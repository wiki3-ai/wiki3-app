import { describe, expect, it } from 'vitest';

import {
  computeReorder,
  defaultNameFromRemote,
  joinPath,
  resolveCloneTarget,
} from '../../src/wiki/dashboard-logic';
import type { Wiki } from '../../src/wiki/types';

function mk(id: string): Wiki {
  return {
    id,
    name: id,
    local_path: null,
    remote: null,
    site_url: `https://example.com/${id}`,
    origin: 'manual',
    description: null,
    created_at: '2026-04-20T00:00:00Z',
    last_opened_at: '2026-04-20T00:00:00Z',
    publish_on_commit: false,
  };
}

const ids = (ws: Wiki[]): string[] => ws.map((w) => w.id);

describe('computeReorder', () => {
  const list = [mk('a'), mk('b'), mk('c'), mk('d')];

  it('moves an item forward with before=true', () => {
    expect(ids(computeReorder(list, 'd', 'b', true))).toEqual(['a', 'd', 'b', 'c']);
  });

  it('moves an item forward with before=false', () => {
    expect(ids(computeReorder(list, 'd', 'b', false))).toEqual(['a', 'b', 'd', 'c']);
  });

  it('moves an item backward with before=true', () => {
    expect(ids(computeReorder(list, 'a', 'c', true))).toEqual(['b', 'a', 'c', 'd']);
  });

  it('moves an item backward with before=false', () => {
    expect(ids(computeReorder(list, 'a', 'c', false))).toEqual(['b', 'c', 'a', 'd']);
  });

  it('is a no-op when src === dst', () => {
    expect(ids(computeReorder(list, 'b', 'b', true))).toEqual(['a', 'b', 'c', 'd']);
  });

  it('is a no-op when ids are unknown', () => {
    expect(ids(computeReorder(list, 'zzz', 'b', true))).toEqual(['a', 'b', 'c', 'd']);
    expect(ids(computeReorder(list, 'a', 'zzz', true))).toEqual(['a', 'b', 'c', 'd']);
  });

  it('does not mutate its input', () => {
    const snapshot = list.slice();
    computeReorder(list, 'a', 'd', false);
    expect(ids(list)).toEqual(ids(snapshot));
  });

  it('handles moving to the end via before=false on last item', () => {
    expect(ids(computeReorder(list, 'b', 'd', false))).toEqual(['a', 'c', 'd', 'b']);
  });

  it('handles moving to the very front via before=true on first item', () => {
    expect(ids(computeReorder(list, 'd', 'a', true))).toEqual(['d', 'a', 'b', 'c']);
  });
});

describe('resolveCloneTarget', () => {
  it('returns cancelled when the picker returned null', () => {
    expect(resolveCloneTarget(null, false, 'x')).toEqual({ kind: 'cancelled' });
  });

  it('uses an empty picked directory directly', () => {
    // This is the bug the previous release shipped: creating a folder
    // in the file dialog and selecting it left an empty folder behind
    // because we prompted for a subfolder and the user hit Cancel.
    expect(resolveCloneTarget('/tmp/new', true, 'my-garden')).toEqual({
      kind: 'use_directly',
      path: '/tmp/new',
    });
  });

  it('falls back to asking for a subfolder when the dir is non-empty', () => {
    expect(resolveCloneTarget('/Users/me/Wiki3', false, 'my-garden')).toEqual({
      kind: 'needs_subfolder',
      parent: '/Users/me/Wiki3',
      defaultName: 'my-garden',
    });
  });
});

describe('joinPath', () => {
  it('uses forward slash on posix-style parents', () => {
    expect(joinPath('/Users/me/Wiki3', 'garden')).toBe('/Users/me/Wiki3/garden');
  });

  it('uses backslash on windows-style parents', () => {
    expect(joinPath('C:\\Users\\me\\Wiki3', 'garden')).toBe('C:\\Users\\me\\Wiki3\\garden');
  });

  it("doesn't double a trailing separator", () => {
    expect(joinPath('/Users/me/Wiki3/', 'garden')).toBe('/Users/me/Wiki3/garden');
    expect(joinPath('C:\\Users\\me\\Wiki3\\', 'garden')).toBe('C:\\Users\\me\\Wiki3\\garden');
  });

  it('trims child whitespace', () => {
    expect(joinPath('/p', '  garden  ')).toBe('/p/garden');
  });
});

describe('defaultNameFromRemote', () => {
  it.each([
    ['https://github.com/owner/repo', 'repo'],
    ['https://github.com/owner/repo.git', 'repo'],
    ['https://github.com/owner/repo/', 'repo'],
    ['git@github.com:owner/repo.git', 'repo'],
    ['', 'my-garden'],
    ['not-a-url', 'my-garden'],
  ])('%s -> %s', (input, expected) => {
    expect(defaultNameFromRemote(input)).toBe(expected);
  });
});
