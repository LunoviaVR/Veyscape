//! Bookmark import from other browsers (read from their local profile files) and from exported
//! files (Netscape bookmark HTML, which every browser can export, or Chromium `Bookmarks` JSON).
//! Nothing is sent anywhere; files are only read.

pub mod favicon;

use crate::errors::{AppError, AppResult};
use crate::storage::ImportNode;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ImportSource {
    pub id: String,
    pub name: String,
}

enum Format {
    ChromiumJson,
    FirefoxPlaces,
}

struct Candidate {
    id: &'static str,
    name: &'static str,
    format: Format,
    path: Option<PathBuf>,
}

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from)
}

fn candidates() -> Vec<Candidate> {
    let local = env_path("LOCALAPPDATA");
    let roaming = env_path("APPDATA");
    let chromium = |rel: &str| {
        local
            .as_ref()
            .map(|l| l.join(rel).join("Default").join("Bookmarks"))
    };
    let opera = roaming.as_ref().and_then(|r| {
        let base = r.join("Opera Software").join("Opera Stable");
        [
            base.join("Default").join("Bookmarks"),
            base.join("Bookmarks"),
        ]
        .into_iter()
        .find(|p| p.is_file())
    });
    vec![
        Candidate {
            id: "chrome",
            name: "Google Chrome",
            format: Format::ChromiumJson,
            path: chromium("Google\\Chrome\\User Data"),
        },
        Candidate {
            id: "edge",
            name: "Microsoft Edge",
            format: Format::ChromiumJson,
            path: chromium("Microsoft\\Edge\\User Data"),
        },
        Candidate {
            id: "brave",
            name: "Brave",
            format: Format::ChromiumJson,
            path: chromium("BraveSoftware\\Brave-Browser\\User Data"),
        },
        Candidate {
            id: "vivaldi",
            name: "Vivaldi",
            format: Format::ChromiumJson,
            path: chromium("Vivaldi\\User Data"),
        },
        Candidate {
            id: "chromium",
            name: "Chromium",
            format: Format::ChromiumJson,
            path: chromium("Chromium\\User Data"),
        },
        Candidate {
            id: "opera",
            name: "Opera",
            format: Format::ChromiumJson,
            path: opera,
        },
        Candidate {
            id: "firefox",
            name: "Firefox",
            format: Format::FirefoxPlaces,
            path: firefox_places(roaming.as_deref()),
        },
    ]
}

/// The most recently used Firefox profile's `places.sqlite`.
fn firefox_places(roaming: Option<&Path>) -> Option<PathBuf> {
    let profiles = roaming?.join("Mozilla").join("Firefox").join("Profiles");
    std::fs::read_dir(profiles)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path().join("places.sqlite")))
        .filter(|p| p.is_file())
        .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
}

/// Browsers with a bookmark file on this machine.
pub fn sources() -> Vec<ImportSource> {
    candidates()
        .into_iter()
        .filter(|c| c.path.as_ref().is_some_and(|p| p.is_file()))
        .map(|c| ImportSource {
            id: c.id.into(),
            name: c.name.into(),
        })
        .collect()
}

/// Reads another browser's bookmarks. Returns (display name, tree).
pub fn read_browser(id: &str) -> AppResult<(String, Vec<ImportNode>)> {
    let c = candidates()
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| AppError::InvalidInput("unknown browser".into()))?;
    let path = c
        .path
        .filter(|p| p.is_file())
        .ok_or_else(|| AppError::NotFound(format!("{} bookmarks", c.name)))?;
    let nodes = match c.format {
        Format::ChromiumJson => parse_chromium_json(&std::fs::read_to_string(&path)?)?,
        Format::FirefoxPlaces => read_firefox(&path)?,
    };
    Ok((c.name.to_string(), nodes))
}

/// Parses an exported file: Netscape bookmark HTML or Chromium JSON.
pub fn parse_file(content: &str) -> AppResult<Vec<ImportNode>> {
    if content.len() > 32 * 1024 * 1024 {
        return Err(AppError::InvalidInput("file is too large".into()));
    }
    let trimmed = content.trim_start_matches('\u{feff}').trim_start();
    if trimmed.starts_with('{') {
        parse_chromium_json(trimmed)
    } else if trimmed.to_ascii_lowercase().contains("<dl") {
        Ok(flatten_toolbar(parse_netscape_html(trimmed)))
    } else {
        Err(AppError::InvalidInput(
            "not a bookmarks file (expected an exported HTML or JSON file)".into(),
        ))
    }
}

fn parse_chromium_json(text: &str) -> AppResult<Vec<ImportNode>> {
    let v: serde_json::Value = serde_json::from_str(text)
        .map_err(|_| AppError::InvalidInput("unreadable bookmarks JSON".into()))?;
    fn node(v: &serde_json::Value) -> Option<ImportNode> {
        let title = v.get("name")?.as_str().unwrap_or_default().to_string();
        match v.get("type")?.as_str()? {
            "url" => Some(ImportNode::Link {
                title,
                url: v.get("url")?.as_str()?.to_string(),
                icon: None,
            }),
            "folder" => Some(ImportNode::Folder {
                title,
                children: children(v),
            }),
            _ => None,
        }
    }
    fn children(v: &serde_json::Value) -> Vec<ImportNode> {
        v.get("children")
            .and_then(|c| c.as_array())
            .map(|a| a.iter().filter_map(node).collect())
            .unwrap_or_default()
    }
    let roots = v
        .get("roots")
        .ok_or_else(|| AppError::InvalidInput("no bookmark roots".into()))?;
    // Bar contents go to the top of the import folder; other roots become subfolders.
    let mut out = roots.get("bookmark_bar").map(children).unwrap_or_default();
    for (key, title) in [("other", "Other bookmarks"), ("synced", "Mobile bookmarks")] {
        let c = roots.get(key).map(children).unwrap_or_default();
        if !c.is_empty() {
            out.push(ImportNode::Folder {
                title: title.into(),
                children: c,
            });
        }
    }
    Ok(out)
}

/// Exports wrap the bar in a "Bookmarks bar" folder; the import folder already sits on our bar,
/// so that level is unwrapped in place.
fn flatten_toolbar(nodes: Vec<ImportNode>) -> Vec<ImportNode> {
    const BAR_NAMES: &[&str] = &[
        "bookmarks bar",
        "bookmarks toolbar",
        "favorites bar",
        "favourites bar",
    ];
    nodes
        .into_iter()
        .flat_map(|n| match n {
            ImportNode::Folder { title, children }
                if BAR_NAMES.contains(&title.trim().to_lowercase().as_str()) =>
            {
                children
            }
            other => vec![other],
        })
        .collect()
}

/// Deepest folder nesting kept from an import (same as the Firefox reader); the tree is inserted
/// and dropped recursively, so a hostile file must not be able to nest without bound.
const MAX_FOLDER_DEPTH: usize = 32;

/// Minimal parser for the Netscape bookmark format (`<DL>`, `<DT><H3>`, `<DT><A HREF>`).
fn parse_netscape_html(html: &str) -> Vec<ImportNode> {
    let lower = html.to_ascii_lowercase();
    // Stack of open folders; the bottom is the file root.
    let mut stack: Vec<(String, Vec<ImportNode>)> = vec![(String::new(), Vec::new())];
    let mut pending_folder: Option<String> = None;
    let mut depth_without_folder = 0usize;
    // `<DL>`s below MAX_FOLDER_DEPTH: their links go into the deepest kept folder.
    let mut too_deep = 0usize;
    let mut i = 0;
    while let Some(off) = lower[i..].find('<') {
        let start = i + off;
        let rest = &lower[start..];
        if rest.starts_with("<h3") {
            let (title, end) = element_text(html, &lower, start, "</h3>");
            pending_folder = Some(title);
            i = end;
        } else if rest.starts_with("<a ") || rest.starts_with("<a\t") {
            let tag_end = lower[start..]
                .find('>')
                .map(|e| start + e + 1)
                .unwrap_or(lower.len());
            let tag = &html[start..tag_end];
            let href = attribute(tag, "href");
            // Chromium and Firefox exports embed each site's icon as a PNG data URL.
            let icon = attribute(tag, "icon");
            let (title, end) = element_text(html, &lower, start, "</a>");
            if let (Some(url), Some(top)) = (href, stack.last_mut()) {
                top.1.push(ImportNode::Link { title, url, icon });
            }
            i = end;
        } else if rest.starts_with("<dl") {
            let folder = pending_folder.take();
            if too_deep > 0 || (folder.is_some() && stack.len() > MAX_FOLDER_DEPTH) {
                too_deep += 1;
            } else if let Some(title) = folder {
                stack.push((title, Vec::new()));
            } else {
                // The outermost <DL> belongs to the root.
                depth_without_folder += 1;
            }
            i = start + 3;
        } else if rest.starts_with("</dl") {
            if too_deep > 0 {
                too_deep -= 1;
            } else if depth_without_folder > 0 && stack.len() == 1 {
                depth_without_folder -= 1;
            } else if stack.len() > 1 {
                let (title, children) = stack.pop().unwrap_or_default();
                if let Some(top) = stack.last_mut() {
                    top.1.push(ImportNode::Folder { title, children });
                }
            }
            i = start + 4;
        } else {
            i = start + 1;
        }
    }
    while stack.len() > 1 {
        let (title, children) = stack.pop().unwrap_or_default();
        if let Some(top) = stack.last_mut() {
            top.1.push(ImportNode::Folder { title, children });
        }
    }
    stack.pop().map(|(_, n)| n).unwrap_or_default()
}

/// Text between the end of the opening tag at `start` and `close`; returns (decoded text, index after close).
fn element_text(html: &str, lower: &str, start: usize, close: &str) -> (String, usize) {
    let open_end = lower[start..]
        .find('>')
        .map(|e| start + e + 1)
        .unwrap_or(lower.len());
    let close_at = lower[open_end..]
        .find(close)
        .map(|e| open_end + e)
        .unwrap_or(lower.len());
    (
        decode_entities(html[open_end..close_at].trim()),
        (close_at + close.len()).min(lower.len()),
    )
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let needle = format!("{name}=");
    // A real attribute starts after whitespace (not inside another value such as `?icon=1`).
    let at = lower
        .match_indices(&needle)
        .find(|(i, _)| lower[..*i].ends_with(|c: char| c.is_ascii_whitespace()))?
        .0
        + needle.len();
    let rest = &tag[at..];
    let value = match rest.chars().next()? {
        q @ ('"' | '\'') => rest[1..].split(q).next()?,
        _ => rest.split([' ', '>']).next()?,
    };
    Some(decode_entities(value))
}

fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&amp;", "&")
}

/// Reads Firefox bookmarks from a copy of `places.sqlite` (Firefox keeps the original locked).
fn read_firefox(places: &Path) -> AppResult<Vec<ImportNode>> {
    let tmp =
        std::env::temp_dir().join(format!("veyscape-places-{}", crate::storage::now_millis()));
    // Everything that writes into `tmp` runs in here so the copy of the user's history is always
    // removed below, even when a copy fails halfway.
    let result = (|| -> AppResult<Vec<ImportNode>> {
        std::fs::create_dir_all(&tmp)?;
        let db_copy = tmp.join("places.sqlite");
        std::fs::copy(places, &db_copy)?;
        let wal = places.with_extension("sqlite-wal");
        if wal.is_file() {
            // Recent changes live in the WAL until Firefox checkpoints it.
            std::fs::copy(&wal, tmp.join("places.sqlite-wal"))?;
        }
        let conn = rusqlite::Connection::open(&db_copy)?;
        let mut stmt = conn.prepare(
            "SELECT b.id, b.parent, b.type, COALESCE(b.title, ''), p.url FROM moz_bookmarks b
             LEFT JOIN moz_places p ON p.id = b.fk ORDER BY b.parent, b.position",
        )?;
        let rows: Vec<(i64, i64, i64, String, Option<String>)> = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<Result<_, _>>()?;
        let root_id = |guid: &str| -> Option<i64> {
            conn.query_row(
                "SELECT id FROM moz_bookmarks WHERE guid = ?1",
                [guid],
                |r| r.get(0),
            )
            .ok()
        };
        fn build(
            rows: &[(i64, i64, i64, String, Option<String>)],
            parent: i64,
            depth: usize,
        ) -> Vec<ImportNode> {
            if depth > 32 {
                return Vec::new();
            }
            rows.iter()
                .filter(|r| r.1 == parent)
                .filter_map(|(id, _, kind, title, url)| match kind {
                    1 => url.clone().map(|url| ImportNode::Link {
                        title: title.clone(),
                        url,
                        icon: None,
                    }),
                    2 => Some(ImportNode::Folder {
                        title: title.clone(),
                        children: build(rows, *id, depth + 1),
                    }),
                    _ => None,
                })
                .collect()
        }
        let mut out = root_id("toolbar_____")
            .map(|id| build(&rows, id, 0))
            .unwrap_or_default();
        for (guid, title) in [
            ("menu________", "Bookmarks menu"),
            ("unfiled_____", "Other bookmarks"),
            // Firefox Sync puts phone bookmarks here.
            ("mobile______", "Mobile bookmarks"),
        ] {
            let children = root_id(guid)
                .map(|id| build(&rows, id, 0))
                .unwrap_or_default();
            if !children.is_empty() {
                out.push(ImportNode::Folder {
                    title: title.into(),
                    children,
                });
            }
        }
        Ok(out)
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(title: &str, url: &str) -> ImportNode {
        ImportNode::Link {
            title: title.into(),
            url: url.into(),
            icon: None,
        }
    }

    #[test]
    fn netscape_html_nested_folders() {
        let html = r#"<!DOCTYPE NETSCAPE-Bookmark-file-1>
<TITLE>Bookmarks</TITLE><H1>Bookmarks</H1>
<DL><p>
  <DT><H3 ADD_DATE="1" PERSONAL_TOOLBAR_FOLDER="true">Bookmarks bar</H3>
  <DL><p>
    <DT><A HREF="https://a.example/" ADD_DATE="1">A &amp; B</A>
    <DT><H3>Dev</H3>
    <DL><p><DT><A HREF='https://docs.example/x?y=1&amp;z=2'>Docs</A></DL><p>
  </DL><p>
  <DT><A HREF="https://loose.example/">Loose</A>
</DL><p>"#;
        let tree = parse_netscape_html(html);
        assert_eq!(
            tree,
            vec![
                ImportNode::Folder {
                    title: "Bookmarks bar".into(),
                    children: vec![
                        link("A & B", "https://a.example/"),
                        ImportNode::Folder {
                            title: "Dev".into(),
                            children: vec![link("Docs", "https://docs.example/x?y=1&z=2")]
                        },
                    ],
                },
                link("Loose", "https://loose.example/"),
            ]
        );
    }

    #[test]
    fn deep_folder_nesting_is_capped() {
        let n = 100_000;
        let html = format!(
            "<DL>{}<DT><A HREF=\"https://deep.example/\">Deep</A>{}<DT><A HREF=\"https://after.example/\">After</A></DL>",
            "<DT><H3>a</H3><DL>".repeat(n),
            "</DL>".repeat(n)
        );
        let tree = parse_netscape_html(&html);
        let (mut depth, mut level) = (0, &tree);
        while let Some(ImportNode::Folder { children, .. }) = level.first() {
            depth += 1;
            level = children;
        }
        assert_eq!(depth, MAX_FOLDER_DEPTH);
        assert_eq!(level, &vec![link("Deep", "https://deep.example/")]);
        assert_eq!(
            tree.len(),
            2,
            "closing tags of skipped levels stay balanced"
        );
        assert_eq!(tree[1], link("After", "https://after.example/"));
    }

    #[test]
    fn firefox_reads_bar_and_mobile_roots() {
        let dir =
            std::env::temp_dir().join(format!("veyscape-ff-test-{}", crate::storage::now_millis()));
        std::fs::create_dir_all(&dir).unwrap();
        let places = dir.join("places.sqlite");
        rusqlite::Connection::open(&places)
            .unwrap()
            .execute_batch(
                "CREATE TABLE moz_places (id INTEGER PRIMARY KEY, url TEXT);
                 CREATE TABLE moz_bookmarks (id INTEGER PRIMARY KEY, type INTEGER, fk INTEGER,
                   parent INTEGER, position INTEGER, title TEXT, guid TEXT);
                 INSERT INTO moz_places VALUES (1, 'https://bar.example/'), (2, 'https://phone.example/');
                 INSERT INTO moz_bookmarks VALUES
                   (1, 2, NULL, 0, 0, NULL, 'root________'),
                   (2, 2, NULL, 1, 0, 'menu', 'menu________'),
                   (3, 2, NULL, 1, 1, 'toolbar', 'toolbar_____'),
                   (5, 2, NULL, 1, 3, 'unfiled', 'unfiled_____'),
                   (6, 2, NULL, 1, 4, 'mobile', 'mobile______'),
                   (10, 1, 1, 3, 0, 'Bar', 'aaaaaaaaaaaa'),
                   (11, 1, 2, 6, 0, 'Phone', 'bbbbbbbbbbbb');",
            )
            .unwrap();
        let tree = read_firefox(&places);
        let missing = read_firefox(&dir.join("missing.sqlite"));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            tree.unwrap(),
            vec![
                link("Bar", "https://bar.example/"),
                ImportNode::Folder {
                    title: "Mobile bookmarks".into(),
                    children: vec![link("Phone", "https://phone.example/")],
                },
            ]
        );
        assert!(missing.is_err());
    }

    #[test]
    fn exported_icons_are_read_but_not_from_inside_other_attributes() {
        let html = r#"<DL><p>
<DT><A HREF="https://a.example/" ICON_URI="https://a.example/f.ico" ICON="data:image/png;base64,AAAA">A</A>
<DT><A HREF="https://b.example/?icon=1">B</A>
</DL>"#;
        let tree = parse_netscape_html(html);
        assert!(
            matches!(&tree[0], ImportNode::Link { icon: Some(i), .. } if i == "data:image/png;base64,AAAA")
        );
        assert!(matches!(&tree[1], ImportNode::Link { icon: None, .. }));
    }

    #[test]
    fn exported_bar_folder_is_unwrapped() {
        let html = "<DL><p><DT><H3>Bookmarks bar</H3><DL><p><DT><A HREF=\"https://a.example/\">A</A></DL><p></DL>";
        assert_eq!(
            parse_file(html).unwrap(),
            vec![link("A", "https://a.example/")]
        );
    }

    #[test]
    fn chromium_json_roots() {
        let json = r#"{"roots":{"bookmark_bar":{"type":"folder","name":"Bar","children":[
            {"type":"url","name":"A","url":"https://a.example/"},
            {"type":"folder","name":"F","children":[{"type":"url","name":"B","url":"https://b.example/"}]}]},
            "other":{"type":"folder","name":"Other","children":[{"type":"url","name":"C","url":"https://c.example/"}]},
            "synced":{"type":"folder","name":"Mobile","children":[]}}}"#;
        let tree = parse_file(json).unwrap();
        assert_eq!(tree.len(), 3);
        assert_eq!(tree[0], link("A", "https://a.example/"));
        assert!(
            matches!(&tree[2], ImportNode::Folder { title, children } if title == "Other bookmarks" && children.len() == 1)
        );
    }

    #[test]
    fn rejects_unrelated_files() {
        assert!(parse_file("hello world").is_err());
        assert!(parse_file("{\"no\":1}").is_err());
    }
}
