<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'
import AuthCard from './AuthCard.vue'
import { api } from '../lib/api'
import { errorText } from '../lib/errors'

const { t } = useI18n()
const route = useRoute()
const state = ref<'busy' | 'ok' | 'error'>('busy')
const error = ref('')

onMounted(async () => {
  try {
    await api.post('/verify-email', { token: route.query.token })
    state.value = 'ok'
  } catch (e) {
    error.value = errorText(e, t)
    state.value = 'error'
  }
})
</script>

<template>
  <AuthCard :title="t('auth.email')">
    <p v-if="state === 'busy'" class="muted">{{ t('auth.verifying') }}</p>
    <p v-else-if="state === 'ok'" class="notice ok">{{ t('auth.verified') }}</p>
    <p v-else class="notice error" role="alert">{{ error }}</p>
    <RouterLink to="/login">{{ t('auth.login') }}</RouterLink>
  </AuthCard>
</template>
