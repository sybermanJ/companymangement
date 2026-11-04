<script setup lang="ts">
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import AuthCard from './AuthCard.vue'
import { api } from '../lib/api'
import { errorText } from '../lib/errors'

const { t } = useI18n()
const email = ref('')
const sent = ref(false)
const error = ref('')

async function submit() {
  error.value = ''
  try {
    await api.post('/password-reset/request', { email: email.value })
    sent.value = true
  } catch (e) {
    error.value = errorText(e, t)
  }
}
</script>

<template>
  <AuthCard :title="t('auth.forgot')">
    <p v-if="sent" class="notice ok">{{ t('auth.resetSent') }}</p>
    <form v-else class="stack" @submit.prevent="submit">
      <label class="field">{{ t('auth.email') }}<input v-model="email" type="email" required autocomplete="email" /></label>
      <p v-if="error" class="notice error" role="alert">{{ error }}</p>
      <button class="primary" type="submit">{{ t('auth.sendReset') }}</button>
    </form>
    <RouterLink to="/login">{{ t('auth.haveAccount') }}</RouterLink>
  </AuthCard>
</template>
