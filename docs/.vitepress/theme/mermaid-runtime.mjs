import mermaid from 'mermaid'

// VitePress owns rendering; disable the library's load listener during module evaluation.
mermaid.startOnLoad = false
mermaid.initialize({ startOnLoad: false })

export async function prepareMermaid(loadLayouts = () => import('@mermaid-js/layout-elk')) {
  const { default: elkLayouts } = await loadLayouts()
  mermaid.registerLayoutLoaders(elkLayouts)
}
