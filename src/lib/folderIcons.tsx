// Icons a bookmark folder can use. Keys must match FOLDER_ICONS in src-tauri/src/storage/bookmarks.rs.
import {
  BookOpen,
  Briefcase,
  Code,
  Coffee,
  Film,
  Folder,
  Gamepad2,
  Globe,
  GraduationCap,
  Heart,
  House,
  Music,
  Newspaper,
  Plane,
  ShoppingCart,
  Sparkles,
  Star,
  Wrench,
  type LucideIcon,
} from 'lucide-react';

export const FOLDER_ICONS: Record<string, { icon: LucideIcon; label: string }> = {
  folder: { icon: Folder, label: 'Folder' },
  star: { icon: Star, label: 'Star' },
  heart: { icon: Heart, label: 'Heart' },
  briefcase: { icon: Briefcase, label: 'Work' },
  book: { icon: BookOpen, label: 'Reading' },
  code: { icon: Code, label: 'Code' },
  music: { icon: Music, label: 'Music' },
  film: { icon: Film, label: 'Video' },
  gamepad: { icon: Gamepad2, label: 'Games' },
  cart: { icon: ShoppingCart, label: 'Shopping' },
  news: { icon: Newspaper, label: 'News' },
  school: { icon: GraduationCap, label: 'Learning' },
  plane: { icon: Plane, label: 'Travel' },
  home: { icon: House, label: 'Home' },
  coffee: { icon: Coffee, label: 'Coffee' },
  globe: { icon: Globe, label: 'Web' },
  wrench: { icon: Wrench, label: 'Tools' },
  sparkles: { icon: Sparkles, label: 'Ideas' },
};

export function FolderIcon({ name, size = 15 }: { name: string | null; size?: number }) {
  const Icon = FOLDER_ICONS[name ?? 'folder']?.icon ?? Folder;
  return <Icon size={size} aria-hidden />;
}
