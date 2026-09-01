import type { Book } from '@/types';
import { formatShelfLabel } from './libraryViewShared';

export type LibraryShelfKey = 'all' | 'essay' | 'research' | 'fiction';

export function getShelfOptions() {
  return [
    { id: 'all' as const, label: formatShelfLabel('all') },
    { id: 'essay' as const, label: formatShelfLabel('essay') },
    { id: 'research' as const, label: formatShelfLabel('research') },
    { id: 'fiction' as const, label: formatShelfLabel('fiction') },
  ];
}

export function filterBooksByShelf(books: Book[], shelf: LibraryShelfKey) {
  if (shelf === 'all') return books;
  return books.filter((book) => {
    const category = book.category.toLowerCase();
    if (shelf === 'essay') return category.includes('essay');
    if (shelf === 'research') return category.includes('research') || category.includes('science');
    return category.includes('fiction') || category.includes('poetry') || category.includes('memoir');
  });
}
