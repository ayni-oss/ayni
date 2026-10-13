import type { Theme } from 'vitepress'
import DefaultTheme from 'vitepress/theme'
import AyniBrand from './AyniBrand.vue'
import Layout from './Layout.vue'
import './style.css'
import { prepareMermaid } from './mermaid-runtime.mjs'

export default {
  extends: DefaultTheme,
  Layout,
  async enhanceApp({ app }) {
    if (!import.meta.env.SSR) await prepareMermaid()
    app.component('AyniBrand', AyniBrand)
  },
} satisfies Theme
