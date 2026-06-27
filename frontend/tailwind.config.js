// TailwindCSS 4.x 主要通过 CSS 内的 @theme 进行配置；
// 此文件保留为兼容入口（v4 中 JS 配置可选）。
export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {},
  },
  plugins: [],
};
