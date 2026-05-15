/**
 * Suggested source URLs for the "Clone from URL…" dashboard dialog.
 *
 * Edit this list to change the suggestions shown in the dropdown.
 * The user can always type a custom URL in addition to picking one
 * from this list.
 */
export interface CloneSource {
  /** Label shown in the dropdown. If omitted, the URL is shown. */
  label?: string;
  /** Remote git URL to clone from. */
  url: string;
}

export const CLONE_SOURCES: CloneSource[] = [
  { label: 'wiki3-ai / agent-client-kernel', url: 'https://github.com/wiki3-ai/agent-client-kernel' },
  { label: 'wiki3-ai / wiki3-ai-site', url: 'https://github.com/wiki3-ai/wiki3-ai-site' },
  { label: 'wiki3-ai / wiki3-ai-template', url: 'https://github.com/wiki3-ai/wiki3-ai-template' },
  { label: 'wiki3-ai / quartz', url: 'https://github.com/wiki3-ai/quartz' },
  { label: 'wiki3-ai / nbdev', url: 'https://github.com/wiki3-ai/nbdev' }
];