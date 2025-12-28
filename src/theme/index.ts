export const colors = {
  // Base
  background: '#1a1a1a',
  foreground: '#e0e0e0',
  muted: '#6b7280',
  dim: '#4b5563',

  // Borders
  border: '#4a4a4a',
  borderFocused: '#888888',
  borderBright: '#aaaaaa',

  // Status indicators
  statusConnected: '#4ade80',
  statusDisconnected: '#6b7280',
  statusError: '#f87171',
  statusConnecting: '#fbbf24',

  // Interactive elements
  selected: '#2563eb',
  selectedText: '#ffffff',

  // Semantic colors
  primary: '#3b82f6',
  danger: '#ef4444',
  success: '#22c55e',
  warning: '#f59e0b',

  // Section colors (for rounded inner boxes)
  sectionBorder: '#4b5563',
  sectionTitle: '#9ca3af',

  // Modal
  modalOverlay: '#0d0d0d',
};

export const layout = {
  borderStyle: 'single' as const,
  modal: {
    defaultHeight: 20,
    defaultWidth: 60,
  },
  padding: {
    medium: 2,
    small: 1,
  },
  sidebarWidth: 38,
};

// Tree icons (refined)
export const icons = {
  // Expand/collapse (smaller, cleaner)
  expanded: '▾',
  collapsed: '▸',

  // Tree structure
  treeBranch: '├─',
  treeEnd: '└─',
  treeVertical: '│ ',
  treeIndent: '  ',

  // Node types
  leaf: '•',

  // Status icons (bracketed for alignment)
  connected: '[●]',
  disconnected: '[○]',
  error: '[!]',
  connecting: '[◐]',

  // Status icons (simple)
  statusDot: '●',
  statusEmpty: '○',
  statusError: '!',
};

// Box drawing characters
export const boxChars = {
  // Standard single-line
  topLeft: '┌',
  topRight: '┐',
  bottomLeft: '└',
  bottomRight: '┘',
  horizontal: '─',
  vertical: '│',

  // Heavy/bold lines (for main panels)
  heavyTopLeft: '┏',
  heavyTopRight: '┓',
  heavyBottomLeft: '┗',
  heavyBottomRight: '┛',
  heavyHorizontal: '━',
  heavyVertical: '┃',

  // Rounded corners (for sections)
  roundTopLeft: '╭',
  roundTopRight: '╮',
  roundBottomLeft: '╰',
  roundBottomRight: '╯',
};

// Keyboard hint formatting
export const keyHints = {
  keyStyle: (key: string) => `${key}`,
  separator: '  ',
};
