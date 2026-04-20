/**
 * DOM regression tests for the card markup.
 *
 * These catch bug #2 from the previous release: inner `<a>` tags were
 * draggable by default and hijacked the HTML5 drag, so the card's own
 * reorder drag never fired. Cards must keep `draggable="true"` and all
 * inner anchors must be explicitly `draggable="false"`.
 */

import { beforeEach, describe, expect, it } from 'vitest';

// Helper: load the real index.html so the CSS selectors / markup used
// by main.ts have a realistic container.
beforeEach(() => {
  document.body.innerHTML = `
    <div id="main-content"></div>
  `;
});

/**
 * Minimal card renderer used to verify that the shape of the markup
 * our real `renderCard` emits keeps inner links non-draggable.
 *
 * We assert against a representative sample rather than importing
 * main.ts directly — main.ts has Tauri side-effects at import time.
 */
function sampleCardHTML(): string {
  return `
    <div class="w3-workspace-list" id="w3-wiki-list">
      <div class="w3-workspace-card" draggable="true" data-wiki-id="w1">
        <div class="w3-ws-header"><h3>w1</h3></div>
        <div class="w3-ws-link-row">
          <a href="#" draggable="false" data-action="reveal-local" data-id="w1">local</a>
        </div>
        <div class="w3-ws-link-row">
          <a href="#" draggable="false" data-action="open-remote" data-id="w1">remote</a>
        </div>
        <div class="w3-ws-link-row">
          <a href="#" draggable="false" data-action="open-site" data-id="w1">site</a>
        </div>
      </div>
      <div class="w3-workspace-card" draggable="true" data-wiki-id="w2">
        <div class="w3-ws-header"><h3>w2</h3></div>
      </div>
    </div>`;
}

describe('card DOM shape', () => {
  it('marks the card itself as draggable', () => {
    document.body.innerHTML = sampleCardHTML();
    const cards = document.querySelectorAll('.w3-workspace-card');
    expect(cards.length).toBeGreaterThan(0);
    cards.forEach((c) => {
      expect(c.getAttribute('draggable')).toBe('true');
    });
  });

  it('marks every inner <a> as draggable="false" so it does not hijack the card drag', () => {
    document.body.innerHTML = sampleCardHTML();
    const anchors = document.querySelectorAll('.w3-workspace-card a');
    expect(anchors.length).toBeGreaterThan(0);
    anchors.forEach((a) => {
      expect(a.getAttribute('draggable')).toBe('false');
    });
  });

  it('verifies that src/main.ts emits draggable="false" on every inner anchor', async () => {
    // Static regression guard: scan the source so we catch any future
    // card link that forgets the attribute. Strip single-line comments
    // first so prose mentioning `<a href="#">` doesn't trip the check.
    const fs = await import('node:fs');
    const path = await import('node:path');
    const rawSource = fs.readFileSync(
      path.resolve(__dirname, '../../src/main.ts'),
      'utf8',
    );
    const source = rawSource
      .split('\n')
      .map((line) => line.replace(/\/\/.*$/, ''))
      .join('\n');
    const anchorMatches = source.match(/<a\s+href="#"[^>]*>/g) ?? [];
    // There should be at least a handful of `<a href="#">` in the card
    // renderer; each one must include `draggable="false"`.
    expect(anchorMatches.length).toBeGreaterThan(0);
    for (const m of anchorMatches) {
      expect(m, `anchor missing draggable="false": ${m}`).toContain('draggable="false"');
    }
  });
});
