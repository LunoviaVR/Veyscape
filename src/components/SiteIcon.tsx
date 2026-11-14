import { Globe } from 'lucide-react';

/** A site's icon (a data URL learned from the page), or a globe until the site has been visited. */
export function SiteIcon({ src, size = 14 }: { src: string | null | undefined; size?: number }) {
  return src ? <img src={src} alt="" width={size} height={size} draggable={false} /> : <Globe size={size} aria-hidden />;
}
