export default {
  brand: '衡图',
  nav: { overview: '总览', projects: '项目', team: '团队', mine: '我的工作', finance: '财务', org: '组织管理', soon: '即将推出' },
  common: { save: '保存', create: '创建', cancel: '取消', logout: '退出登录', language: '语言', theme: '主题', light: '浅色', dark: '深色', loading: '加载中…', none: '无', error: '出错了：{msg}' },
  auth: {
    login: '登录', signup: '注册公司', email: '邮箱', password: '密码', newPassword: '新密码', name: '您的姓名', company: '公司名称',
    slug: '公司标识（用于网址）', currency: '默认币种', noAccount: '还没有账号？注册公司', haveAccount: '已有账号？登录',
    forgot: '忘记密码？', sendReset: '发送重置邮件', resetSent: '如果该邮箱已注册，我们已发送重置链接。', resetDone: '密码已重置，请重新登录。',
    setPassword: '设置密码', verified: '邮箱已验证。', verifying: '正在验证…', badLink: '链接无效或已过期。',
    unverified: '您的邮箱尚未验证。', resend: '重新发送验证邮件', resent: '验证邮件已发送。', signedUp: '注册成功，验证邮件已发送，请登录。',
    acceptTitle: '接受邀请', accept: '加入公司', accepted: '已加入，请登录。', passwordHint: '至少 10 个字符', tooMany: '尝试次数过多，请 15 分钟后再试。', wrong: '邮箱或密码错误。',
  },
  org: {
    title: '组织管理', departments: '部门', members: '成员', roles: '角色',
    deptName: '部门名称', parent: '上级部门', head: '负责人', addDept: '添加部门', noHead: '未设置',
    choose: '请选择', inviteeName: '姓名', invite: '邀请成员', inviteSent: '邀请邮件已发送给 {email}。', admin: '管理员', finance: '财务', status: '状态',
    statusActive: '在职', statusInvited: '已邀请', statusDeactivated: '已停用',
    roleName: '角色名称', roleDept: '所属部门（可选）', addRole: '添加角色', assign: '分配角色', person: '成员', companyWide: '全公司',
    adminOnly: '只有管理员可以修改组织设置。',
  },
}
