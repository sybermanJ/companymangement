export default {
  brand: '衡圖',
  nav: { overview: '總覽', projects: '專案', team: '團隊', mine: '我的工作', finance: '財務', org: '組織管理', soon: '即將推出' },
  common: { save: '儲存', create: '建立', cancel: '取消', logout: '登出', language: '語言', theme: '主題', light: '淺色', dark: '深色', loading: '載入中…', none: '無', error: '發生錯誤：{msg}' },
  auth: {
    login: '登入', signup: '註冊公司', email: '電子郵件', password: '密碼', newPassword: '新密碼', name: '您的姓名', company: '公司名稱',
    slug: '公司識別（用於網址）', currency: '預設幣別', noAccount: '還沒有帳號？註冊公司', haveAccount: '已有帳號？登入',
    forgot: '忘記密碼？', sendReset: '傳送重設郵件', resetSent: '若此信箱已註冊，我們已寄出重設連結。', resetDone: '密碼已重設，請重新登入。',
    setPassword: '設定密碼', verified: '信箱已驗證。', verifying: '正在驗證…', badLink: '連結無效或已過期。',
    unverified: '您的信箱尚未驗證。', resend: '重新寄送驗證信', resent: '驗證信已寄出。', signedUp: '註冊成功，驗證信已寄出，請登入。',
    acceptTitle: '接受邀請', accept: '加入公司', accepted: '已加入，請登入。', passwordHint: '至少 10 個字元', tooMany: '嘗試次數過多，請 15 分鐘後再試。', wrong: '信箱或密碼錯誤。',
  },
  org: {
    title: '組織管理', departments: '部門', members: '成員', roles: '角色',
    deptName: '部門名稱', parent: '上級部門', head: '負責人', addDept: '新增部門', noHead: '未設定',
    choose: '請選擇', inviteeName: '姓名', invite: '邀請成員', inviteSent: '邀請信已寄給 {email}。', admin: '管理員', finance: '財務', status: '狀態',
    statusActive: '在職', statusInvited: '已邀請', statusDeactivated: '已停用',
    roleName: '角色名稱', roleDept: '所屬部門（選填）', addRole: '新增角色', assign: '指派角色', person: '成員', companyWide: '全公司',
    adminOnly: '只有管理員可以修改組織設定。',
  },
}
