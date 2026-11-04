<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { LOCALES, setLocale, type Locale } from '../i18n'
defineProps<{ title: string }>()
const { t, locale } = useI18n()
</script>

<template>
  <div class="auth-page">
    <div class="card stack">
      <div class="row" style="justify-content: space-between">
        <span class="brand">{{ t('brand') }}</span>
        <select :aria-label="t('common.language')" :value="locale" @change="setLocale(($event.target as HTMLSelectElement).value as Locale)">
          <option v-for="l in LOCALES" :key="l.code" :value="l.code">{{ l.label }}</option>
        </select>
      </div>
      <h1>{{ title }}</h1>
      <slot />
    </div>
  </div>
</template>
