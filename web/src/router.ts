import { createRouter, createWebHistory } from 'vue-router'
import { loadMe, me } from './lib/session'

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', redirect: '/projects' },
    { path: '/login', component: () => import('./pages/Login.vue'), meta: { public: true } },
    { path: '/signup', component: () => import('./pages/Signup.vue'), meta: { public: true } },
    { path: '/forgot-password', component: () => import('./pages/ForgotPassword.vue'), meta: { public: true } },
    { path: '/reset-password', component: () => import('./pages/ResetPassword.vue'), meta: { public: true } },
    { path: '/verify-email', component: () => import('./pages/VerifyEmail.vue'), meta: { public: true } },
    { path: '/accept-invite', component: () => import('./pages/AcceptInvite.vue'), meta: { public: true } },
    { path: '/org', component: () => import('./pages/Org.vue') },
    { path: '/projects', component: () => import('./pages/Projects.vue') },
    { path: '/projects/:id', component: () => import('./pages/Project.vue') },
    { path: '/my', component: () => import('./pages/Mine.vue') },
    { path: '/inbox', component: () => import('./pages/Inbox.vue') },
    { path: '/routines', component: () => import('./pages/Routines.vue') },
    { path: '/requests', component: () => import('./pages/Requests.vue') },
    { path: '/contracts', component: () => import('./pages/Contracts.vue') },
    { path: '/finance', component: () => import('./pages/Finance.vue') },
    { path: '/overview', component: () => import('./pages/Overview.vue') },
    { path: '/team', component: () => import('./pages/Team.vue') },
  ],
})

router.beforeEach(async (to) => {
  if (to.meta.public) return true
  if (!me.value) await loadMe()
  return me.value ? true : { path: '/login', query: { next: to.fullPath } }
})
