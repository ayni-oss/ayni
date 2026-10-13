import assert from 'node:assert/strict'
import test from 'node:test'

test('browser load cannot scan diagram containers while the ELK import is pending', async (t) => {
  const listeners = new Map()
  const previousWindow = globalThis.window
  const previousDocument = globalThis.document
  globalThis.window = { addEventListener: (event, callback) => listeners.set(event, callback) }
  globalThis.document = {}
  t.after(() => {
    globalThis.window = previousWindow
    globalThis.document = previousDocument
  })

  const { default: mermaid } = await import('mermaid')
  const scan = t.mock.method(mermaid, 'run', async () => {})
  const pageLoad = listeners.get('load')
  assert.equal(typeof pageLoad, 'function', 'exercise Mermaid’s actual browser load listener')

  // Confirm that the library's default would start the competing DOM scanner.
  pageLoad()
  assert.equal(scan.mock.callCount(), 1)
  scan.mock.resetCalls()

  const { prepareMermaid } = await import('../../docs/.vitepress/theme/mermaid-runtime.mjs')

  let releaseLayouts
  let layoutsRequested
  const layoutsPending = new Promise(resolve => { releaseLayouts = resolve })
  const requestObserved = new Promise(resolve => { layoutsRequested = resolve })
  const preparation = prepareMermaid(() => { layoutsRequested(); return layoutsPending })
  await requestObserved
  pageLoad()
  assert.equal(scan.mock.callCount(), 0, 'load must not parse empty component containers')

  releaseLayouts(await import('@mermaid-js/layout-elk'))
  await preparation
  assert.equal(mermaid.mermaidAPI.getConfig().startOnLoad, false)
  pageLoad()
  assert.equal(scan.mock.callCount(), 0, 'components remain the only diagram renderer')
})
