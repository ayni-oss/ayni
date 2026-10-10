<script setup lang="ts">
import DefaultTheme from 'vitepress/theme'
import { useData } from 'vitepress'
import AyniBrand from './AyniBrand.vue'

const { frontmatter } = useData()
</script>

<template>
  <DefaultTheme.Layout>
    <template #nav-bar-title-before>
      <AyniBrand />
    </template>
    <template #home-hero-info>
      <h1 class="ayni-home-heading">
        <AyniBrand class="ayni-home-brand" />
        <span>{{ frontmatter.hero.text }}</span>
      </h1>
      <p class="ayni-home-tagline">{{ frontmatter.hero.tagline }}</p>
    </template>
  </DefaultTheme.Layout>
</template>
