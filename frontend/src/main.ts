// Vue 应用入口：挂载根组件 + 引入全局样式

import { createApp } from 'vue';
import App from './App.vue';
import './styles/index.css';

const rootEl = document.getElementById('root');
if (!rootEl) throw new Error('#root element not found');

createApp(App).mount(rootEl);
