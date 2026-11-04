<script setup lang="ts">
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import AuthCard from './AuthCard.vue'
import { api } from '../lib/api'
import { errorText } from '../lib/errors'

const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const password = ref('')
const error = ref('')

async function submit() {
  error.value = ''
  try {
    await api.post('/password-reset/confirm', { token: route.query.token, password: password.value })
    router.push({ path: '/login', query: { msg: 'resetDone' } })
  } catch (e) {
    error.value = errorText(e, t)
  }
}
</script>

<template>
  <AuthCard :title="t('auth.setPassword')">
    <form class="stack" @submit.prevent="submit">
      <label class="field">{{ t('auth.newPassword') }} · {{ t('auth.passwordHint') }}<input v-model="password" type="password" minlength="10" required autocomplete="new-password" /></label>
      <p v-if="error" class="notice error" role="alert">{{ error }}</p>
      <button class="primary" type="submit">{{ t('auth.setPassword') }}</button>
    </form>
  </AuthCard>
</template>
