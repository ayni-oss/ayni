<script setup lang="ts">
import { withBase } from 'vitepress'

const logoMask = `url("${withBase('/ayni-logo.svg')}")`
</script>

<template>
  <span class="ayni-brand">
    <span
      class="ayni-brand-mark"
      aria-hidden="true"
      :style="{ '--ayni-logo-mask': logoMask }"
    />
    <span>Ayni</span>
  </span>
</template>

<style scoped>
.ayni-brand {
  display: inline-flex;
  align-items: center;
  gap: 0.35em;
  font: inherit;
  white-space: nowrap;
}

.ayni-brand-mark {
  flex: none;
  width: 1em;
  height: 1em;
  background-color: currentColor;
  mask: var(--ayni-logo-mask) center / contain no-repeat;
  -webkit-mask: var(--ayni-logo-mask) center / contain no-repeat;
}
</style>
