import { describe, expect, it } from 'vitest'
import { getLinkIconKind } from '../src'

describe('getLinkIconKind', () => {
  it('classifies GitHub hosts', () => {
    expect(getLinkIconKind('https://github.com/arcships/dim-electron')).toBe('github')
    expect(getLinkIconKind('https://gist.github.com/user/123')).toBe('github')
    expect(getLinkIconKind('HTTPS://GITHUB.COM/a/b')).toBe('github')
  })

  it('classifies public and self-hosted GitLab hosts', () => {
    expect(getLinkIconKind('https://gitlab.com/group/project')).toBe('gitlab')
    expect(getLinkIconKind('https://sub.gitlab.com/group/project')).toBe('gitlab')
    expect(getLinkIconKind('https://gitlab.example.com/group/project')).toBe('gitlab')
  })

  it('returns null for generic, invalid, or non-http links', () => {
    expect(getLinkIconKind('https://example.com/a')).toBeNull()
    expect(getLinkIconKind('mailto:dev@example.com')).toBeNull()
    expect(getLinkIconKind('not a url')).toBeNull()
    expect(getLinkIconKind('/workspace/src/app.ts')).toBeNull()
  })

  it('does not classify hosts that merely embed a provider name', () => {
    expect(getLinkIconKind('https://notgithub.com/a/b')).toBeNull()
    expect(getLinkIconKind('https://mygitlab.example.com/a/b')).toBeNull()
  })
})
