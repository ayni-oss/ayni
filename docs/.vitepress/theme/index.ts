import type { Theme } from 'vitepress'
import DefaultTheme from 'vitepress/theme'
import AyniBrand from './AyniBrand.vue'
import Layout from './Layout.vue'
import './style.css'

export default {
  extends: DefaultTheme,
  Layout,
  enhanceApp({ app }) {
    app.component('AyniBrand', AyniBrand)
  },
} satisfies Theme
