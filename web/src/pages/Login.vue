<script setup lang="ts">
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import AuthCard from './AuthCard.vue'
import { api } from '../lib/api'
import { loadMe } from '../lib/session'
import { errorText } from '../lib/errors'

const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const email = ref('')
const password = ref('')
const error = ref('')
const busy = ref(false)
const info = ref(route.query.msg ? t(`auth.${route.query.msg}`) : '')

async function submit() {
  busy.value = true
  error.value = ''
  try {
    await api.post('/login', { email: email.value, password: password.value })
    await loadMe()
    router.push(typeof route.query.next === 'string' ? route.query.next : '/projects')
  } catch (e) {
    error.value = errorText(e, t)
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <AuthCard :title="t('auth.login')">
    <p v-if="info" class="notice ok">{{ info }}</p>
    <form class="stack" @submit.prevent="submit">
      <label class="field">{{ t('auth.email') }}<input v-model="email" type="email" autocomplete="email" required /></label>
      <label class="field">{{ t('auth.password') }}<input v-model="password" type="password" autocomplete="current-password" required /></label>
      <p v-if="error" class="notice error" role="alert">{{ error }}</p>
      <button class="primary" type="submit" :disabled="busy">{{ t('auth.login') }}</button>
    </form>
    <div class="row" style="justify-content: space-between">
      <RouterLink to="/signup">{{ t('auth.noAccount') }}</RouterLink>
      <RouterLink to="/forgot-password">{{ t('auth.forgot') }}</RouterLink>
    </div>
  </AuthCard>
</template>
