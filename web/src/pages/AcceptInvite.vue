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
    await api.post('/invitations/accept', { token: route.query.token, password: password.value || undefined })
    router.push({ path: '/login', query: { msg: 'accepted' } })
  } catch (e) {
    error.value = errorText(e, t)
  }
}
</script>

<template>
  <AuthCard :title="t('auth.acceptTitle')">
    <form class="stack" @submit.prevent="submit">
      <label class="field">{{ t('auth.setPassword') }} · {{ t('auth.passwordHint') }}<input v-model="password" type="password" minlength="10" autocomplete="new-password" /></label>
      <p v-if="error" class="notice error" role="alert">{{ error }}</p>
      <button class="primary" type="submit">{{ t('auth.accept') }}</button>
    </form>
  </AuthCard>
</template>
