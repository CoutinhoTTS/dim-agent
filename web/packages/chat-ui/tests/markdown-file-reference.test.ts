import { afterEach, describe, expect, it } from 'vitest'
import {
  isAbsoluteLocalPath,
  normalizeWindowsDrivePath,
  pathLooksLikeDirectory,
  resolveMarkdownFileReference,
  resolveRelativeWorkspaceLink,
  resolveRelativeWorkspaceLinkFromFile,
  resolveWorkspaceFilePath,
} from '../src'

const originalPlatform = Object.getOwnPropertyDescriptor(navigator, 'platform')
const originalUserAgent = Object.getOwnPropertyDescriptor(navigator, 'userAgent')

function mockWindowsRuntime() {
  Object.defineProperty(navigator, 'platform', { value: 'Win32', configurable: true })
  Object.defineProperty(navigator, 'userAgent', { value: 'Mozilla/5.0 (Windows NT 10.0)', configurable: true })
}

afterEach(() => {
  if (originalPlatform) {
    Object.defineProperty(navigator, 'platform', originalPlatform)
  } else {
    // @ts-expect-error allow deleting the prototype-backed property
    delete navigator.platform
  }
  if (originalUserAgent) {
    Object.defineProperty(navigator, 'userAgent', originalUserAgent)
  } else {
    // @ts-expect-error allow deleting the prototype-backed property
    delete navigator.userAgent
  }
})

describe('normalizeWindowsDrivePath', () => {
  it('strips the leading slash from a POSIX-style Windows drive path on Windows', () => {
    mockWindowsRuntime()
    expect(normalizeWindowsDrivePath('/C:/Users/18616/artifacts/image.png')).toBe(
      'C:/Users/18616/artifacts/image.png',
    )
    expect(normalizeWindowsDrivePath('/C:\\Users\\18616\\artifacts\\image.png')).toBe(
      'C:\\Users\\18616\\artifacts\\image.png',
    )
  })

  it('leaves non-drive paths untouched on Windows', () => {
    mockWindowsRuntime()
    expect(normalizeWindowsDrivePath('/Users/Simon/artifacts/image.png')).toBe(
      '/Users/Simon/artifacts/image.png',
    )
    expect(normalizeWindowsDrivePath('C:/Users/18616/artifacts/image.png')).toBe(
      'C:/Users/18616/artifacts/image.png',
    )
  })
})

describe('resolveMarkdownFileReference on Windows', () => {
  it('resolves /C:/... markdown image destinations to a Windows absolute path', () => {
    mockWindowsRuntime()
    const result = resolveMarkdownFileReference('/C:/Users/18616/dim-agent/artifacts/image-mse9bwla.png')
    expect(result).toEqual({
      absolutePath: 'C:/Users/18616/dim-agent/artifacts/image-mse9bwla.png',
    })
    expect(isAbsoluteLocalPath(result?.absolutePath ?? '')).toBe(true)
  })

  it('resolves the plain C:/... and C:\\... forms on Windows', () => {
    mockWindowsRuntime()
    expect(resolveMarkdownFileReference('C:/Users/18616/dim-agent/artifacts/image.png')).toEqual({
      absolutePath: 'C:/Users/18616/dim-agent/artifacts/image.png',
    })
    // Backslashes in drive paths are normalized to forward slashes.
    expect(resolveMarkdownFileReference('C:\\Users\\18616\\dim-agent\\artifacts\\image.png')).toEqual({
      absolutePath: 'C:/Users/18616/dim-agent/artifacts/image.png',
    })
  })

  it('resolves file:///C:/... on Windows', () => {
    mockWindowsRuntime()
    expect(resolveMarkdownFileReference('file:///C:/Users/18616/dim-agent/artifacts/image.png')).toEqual({
      absolutePath: 'C:/Users/18616/dim-agent/artifacts/image.png',
    })
  })

  it('resolves percent-encoded UNC destinations on Windows', () => {
    mockWindowsRuntime()
    expect(resolveMarkdownFileReference('%5C%5Cserver%5Cshare%5C_site%5Cindex.html')).toEqual({
      absolutePath: '\\\\server\\share\\_site\\index.html',
    })
  })

  it('resolves file://server/share UNC URLs on Windows', () => {
    mockWindowsRuntime()
    expect(resolveMarkdownFileReference('file://server/share/_site/index.html')).toEqual({
      absolutePath: '\\\\server\\share\\_site\\index.html',
    })
  })

  it('does not treat protocol-relative URLs as UNC on Windows', () => {
    mockWindowsRuntime()
    expect(resolveMarkdownFileReference('//cdn.example.com/a.png')).toBeNull()
  })

  it('keeps /C:/... untouched on non-Windows runtimes', () => {
    Object.defineProperty(navigator, 'platform', { value: 'MacIntel', configurable: true })
    Object.defineProperty(navigator, 'userAgent', { value: 'Mozilla/5.0 (Macintosh)', configurable: true })
    expect(resolveMarkdownFileReference('/C:/Users/18616/dim-agent/artifacts/image.png')).toEqual({
      absolutePath: '/C:/Users/18616/dim-agent/artifacts/image.png',
    })
  })
})

describe('resolveRelativeWorkspaceLinkFromFile', () => {
  it('returns the normalized absolute path for /C:/... hrefs on Windows', () => {
    mockWindowsRuntime()
    expect(resolveRelativeWorkspaceLinkFromFile(
      '/C:/Users/18616/dim-agent/artifacts/image.png',
      'C:/dim-agent/README.md',
    )).toBe('C:/Users/18616/dim-agent/artifacts/image.png')
  })

  it('returns the absolute path unchanged on Windows', () => {
    mockWindowsRuntime()
    expect(resolveRelativeWorkspaceLinkFromFile(
      'C:/Users/18616/dim-agent/artifacts/image.png',
      'C:/dim-agent/README.md',
    )).toBe('C:/Users/18616/dim-agent/artifacts/image.png')
  })

  it('resolves ./C:/... and .\\C:\\... hrefs to the normalized absolute path', () => {
    mockWindowsRuntime()
    expect(resolveRelativeWorkspaceLinkFromFile(
      './C:/Users/18616/dim-agent/artifacts/image.png',
      'C:/dim-agent/README.md',
    )).toBe('C:/Users/18616/dim-agent/artifacts/image.png')
    expect(resolveRelativeWorkspaceLinkFromFile(
      '.\\C:\\Users\\18616\\dim-agent\\artifacts\\image.png',
      'C:/dim-agent/README.md',
    )).toBe('C:/Users/18616/dim-agent/artifacts/image.png')
  })

  it('keeps UNC hrefs untouched on Windows', () => {
    mockWindowsRuntime()
    expect(resolveRelativeWorkspaceLinkFromFile(
      '\\\\server\\share\\artifacts\\image.png',
      'C:/dim-agent/README.md',
    )).toBe('\\\\server\\share\\artifacts\\image.png')
  })

  it('joins relative hrefs against the source file directory', () => {
    mockWindowsRuntime()
    expect(resolveRelativeWorkspaceLinkFromFile(
      'artifacts/image.png',
      'C:/dim-agent/README.md',
    )).toBe('C:/dim-agent/artifacts/image.png')
  })
})

describe('resolveRelativeWorkspaceLink', () => {
  it('normalizes workspace-relative hrefs and strips a leading ./', () => {
    expect(resolveRelativeWorkspaceLink('./docs/guide.md')).toBe('docs/guide.md')
    expect(resolveRelativeWorkspaceLink('docs%20dir/guide.md')).toBe('docs dir/guide.md')
    expect(resolveRelativeWorkspaceLink('docs\\guide.md')).toBe('docs/guide.md')
  })

  it('rejects anchors, protocol-relative URLs, and URI schemes', () => {
    expect(resolveRelativeWorkspaceLink('#section')).toBeNull()
    expect(resolveRelativeWorkspaceLink('//cdn.example.com/a.png')).toBeNull()
    expect(resolveRelativeWorkspaceLink('https://example.com/a.png')).toBeNull()
    expect(resolveRelativeWorkspaceLink('mailto:dev@example.com')).toBeNull()
    expect(resolveRelativeWorkspaceLink('')).toBeNull()
  })
})

describe('pathLooksLikeDirectory', () => {
  it('classifies extension-less segments as directories', () => {
    expect(pathLooksLikeDirectory('/repo/src/components')).toBe(true)
    expect(pathLooksLikeDirectory('src/')).toBe(true)
    expect(pathLooksLikeDirectory('/repo/src/app.ts')).toBe(false)
    expect(pathLooksLikeDirectory('C:\\repo\\src\\app.vue:42')).toBe(false)
  })
})

describe('resolveWorkspaceFilePath', () => {
  it('joins a relative path against an ordinary cwd', () => {
    expect(resolveWorkspaceFilePath('/tmp/vue-tui', 'docs/assets/x.png')).toBe(
      '/tmp/vue-tui/docs/assets/x.png',
    )
    expect(resolveWorkspaceFilePath('/tmp/vue-tui/', './docs/assets/x.png')).toBe(
      '/tmp/vue-tui/docs/assets/x.png',
    )
  })

  it('keeps the POSIX root `/` as the workspace cwd', () => {
    expect(resolveWorkspaceFilePath('/', 'docs/assets/x.png')).toBe('/docs/assets/x.png')
    expect(resolveWorkspaceFilePath('/', './assets/x.png')).toBe('/assets/x.png')
  })

  it('passes absolute inputs through unchanged', () => {
    expect(resolveWorkspaceFilePath('/tmp/vue-tui', '/etc/hosts')).toBe('/etc/hosts')
  })

  it('keeps Windows drive and UNC roots on Windows', () => {
    mockWindowsRuntime()
    expect(resolveWorkspaceFilePath('C:\\', 'docs/assets/x.png')).toBe('C:/docs/assets/x.png')
    expect(resolveWorkspaceFilePath('C:\\proj\\', 'docs/assets/x.png')).toBe(
      'C:/proj/docs/assets/x.png',
    )
    expect(resolveWorkspaceFilePath('\\\\server\\share', 'docs/assets/x.png')).toBe(
      '\\\\server\\share/docs/assets/x.png',
    )
  })

  it('throws when no cwd is available and the input is relative', () => {
    expect(() => resolveWorkspaceFilePath(null, 'docs/assets/x.png')).toThrow()
    expect(() => resolveWorkspaceFilePath('', 'docs/assets/x.png')).toThrow()
  })
})
