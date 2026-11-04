export default {
  brand: '衡図',
  nav: { overview: '概要', projects: 'プロジェクト', team: 'チーム', mine: '自分の作業', finance: '経理', org: '組織管理', soon: '近日公開' },
  common: { save: '保存', create: '作成', cancel: 'キャンセル', logout: 'ログアウト', language: '言語', theme: 'テーマ', light: 'ライト', dark: 'ダーク', loading: '読み込み中…', none: 'なし', error: 'エラーが発生しました：{msg}' },
  auth: {
    login: 'ログイン', signup: '会社を登録', email: 'メールアドレス', password: 'パスワード', newPassword: '新しいパスワード', name: 'お名前', company: '会社名',
    slug: '会社ID（URL に使用）', currency: '既定の通貨', noAccount: 'アカウントがない場合は会社を登録', haveAccount: 'アカウントをお持ちの方はログイン',
    forgot: 'パスワードをお忘れですか？', sendReset: 'リセットメールを送信', resetSent: '登録済みのアドレスであれば、リセット用リンクを送信しました。', resetDone: 'パスワードを再設定しました。ログインしてください。',
    setPassword: 'パスワードを設定', verified: 'メールアドレスを確認しました。', verifying: '確認中…', badLink: 'リンクが無効か期限切れです。',
    unverified: 'メールアドレスが未確認です。', resend: '確認メールを再送', resent: '確認メールを送信しました。', signedUp: '登録しました。確認メールを送信したのでログインしてください。',
    acceptTitle: '招待を承諾', accept: '会社に参加', accepted: '参加しました。ログインしてください。', passwordHint: '10 文字以上', tooMany: '試行回数が多すぎます。15 分後に再試行してください。', wrong: 'メールアドレスまたはパスワードが違います。',
  },
  org: {
    title: '組織管理', departments: '部署', members: 'メンバー', roles: '役割',
    deptName: '部署名', parent: '上位部署', head: '責任者', addDept: '部署を追加', noHead: '未設定',
    choose: '選択してください', inviteeName: '氏名', invite: 'メンバーを招待', inviteSent: '{email} に招待メールを送信しました。', admin: '管理者', finance: '経理', status: '状態',
    statusActive: '在籍', statusInvited: '招待中', statusDeactivated: '無効',
    roleName: '役割名', roleDept: '所属部署（任意）', addRole: '役割を追加', assign: '役割を割り当て', person: 'メンバー', companyWide: '全社',
    adminOnly: '組織設定を変更できるのは管理者のみです。',
  },
}
