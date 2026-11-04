<script setup lang="ts">
import { reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import AuthCard from './AuthCard.vue'
import { api } from '../lib/api'
import { errorText } from '../lib/errors'

const { t } = useI18n()
const router = useRouter()
const f = reactive({ company_name: '', slug: '', default_currency: 'CNY', name: '', email: '', password: '' })
const error = ref('')
const busy = ref(false)
const slugEdited = ref(false)

watch(() => f.company_name, (v) => {
  if (!slugEdited.value) f.slug = v.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 63)
})

async function submit() {
  busy.value = true
  error.value = ''
  try {
    await api.post('/signup', f)
    router.push({ path: '/login', query: { msg: 'signedUp' } })
  } catch (e) {
    error.value = errorText(e, t)
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <AuthCard :title="t('auth.signup')">
    <form class="stack" @submit.prevent="submit">
      <label class="field">{{ t('auth.company') }}<input v-model="f.company_name" required maxlength="100" /></label>
      <label class="field">{{ t('auth.slug') }}<input v-model="f.slug" required pattern="[a-z0-9][a-z0-9\-]{1,62}" @input="slugEdited = true" /></label>
      <label class="field">{{ t('auth.currency') }}
        <select v-model="f.default_currency">
          <option v-for="c in ['CNY', 'USD', 'EUR', 'JPY', 'HKD', 'TWD', 'SGD', 'GBP']" :key="c" :value="c">{{ c }}</option>
        </select>
      </label>
      <label class="field">{{ t('auth.name') }}<input v-model="f.name" required maxlength="100" autocomplete="name" /></label>
      <label class="field">{{ t('auth.email') }}<input v-model="f.email" type="email" required autocomplete="email" /></label>
      <label class="field">{{ t('auth.password') }} · {{ t('auth.passwordHint') }}<input v-model="f.password" type="password" minlength="10" required autocomplete="new-password" /></label>
      <p v-if="error" class="notice error" role="alert">{{ error }}</p>
      <button class="primary" type="submit" :disabled="busy">{{ t('auth.signup') }}</button>
    </form>
    <RouterLink to="/login">{{ t('auth.haveAccount') }}</RouterLink>
  </AuthCard>
</template>
