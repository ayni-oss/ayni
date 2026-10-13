<script setup lang="ts">
import source from '../../public/ayni-logo.svg?raw'

withDefaults(defineProps<{ label?: string }>(), { label: 'Ayni' })
const logo = source.replaceAll('fill="black"', 'fill="currentColor"').replaceAll('stroke="black"', 'stroke="currentColor"')
</script>

<template>
  <span class="ayni-brand">
    <span class="ayni-brand-mark" aria-hidden="true" v-html="logo" />
    <span>{{ label }}</span>
  </span>
</template>

<style scoped>
.ayni-brand {
  display: inline-flex;
  align-items: center;
  gap: 0.35em;
  font: inherit;
}
.ayni-brand-mark { flex: none; width: 1em; height: 1em; }
.ayni-brand-mark :deep(svg) { display: block; width: 100%; height: 100%; }
</style>
