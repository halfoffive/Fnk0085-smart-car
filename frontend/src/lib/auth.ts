// 前端访问密码登录态管理
// sessionStorage 生命周期 = tab，关 tab 自动失效需重登

const AUTH_KEY = 'fnk_authed';

/** 是否已登录（读 sessionStorage） */
export function isAuthed(): boolean {
  return sessionStorage.getItem(AUTH_KEY) === '1';
}

/** 标记已登录（登录成功后调用） */
export function setAuthed(): void {
  sessionStorage.setItem(AUTH_KEY, '1');
}

/** 清除登录态（退出登录或 401 时调用） */
export function clearAuthed(): void {
  sessionStorage.removeItem(AUTH_KEY);
}
