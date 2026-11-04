import { createApp } from 'vue'
import App from './App.vue'
import { router } from './router'
import { i18n } from './i18n'
import { applyTheme } from './lib/session'
import './styles.css'

applyTheme()
document.documentElement.lang = i18n.global.locale.value
createApp(App).use(i18n).use(router).mount('#app')
