import { isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import './island.css';

type AssistantEvent = {kind: string; text: string; level: number};

export async function startIsland() {
  document.documentElement.classList.add('island-page');
  document.body.innerHTML = `<div class="island-pill" role="status" aria-live="polite">
    <div class="island-symbol"><svg width="23" height="23" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="9" y="2" width="6" height="12" rx="3"/><path d="M5 10v2a7 7 0 0014 0v-2M12 19v3M8 22h8"/></svg></div>
    <div class="island-content"><strong id="island-title">Вася слушает</strong><span id="island-detail">Произнесите команду</span></div>
    <div class="island-meter" aria-hidden="true">${Array.from({length:7}, () => '<i></i>').join('')}</div>
  </div>`;
  const pill = document.querySelector<HTMLElement>('.island-pill')!;
  const title = document.getElementById('island-title')!;
  const detail = document.getElementById('island-detail')!;
  if (!isTauri()) { detail.textContent = 'Предпросмотр панели активации'; return; }
  await listen<AssistantEvent>('assistant', ({payload: event}) => {
    if (event.kind === 'level') {
      document.querySelectorAll<HTMLElement>('.island-meter i').forEach((bar, i) => {
        bar.style.height = `${5 + Math.min(1, event.level) * (18 + Math.abs(Math.sin(i * 1.9)) * 15)}px`;
      });
      return;
    }
    if (event.kind === 'activated') {
      pill.className = 'island-pill'; title.textContent = 'Вася слушает'; detail.textContent = 'Произнесите команду';
    } else if (event.kind === 'heard') {
      detail.textContent = event.text; title.textContent = 'Выполняю команду'; pill.className = 'island-pill thinking';
    } else if (event.kind === 'status') {
      if (event.text.includes('модели')) { title.textContent = 'Вася думает'; pill.className = 'island-pill thinking'; }
      else if (event.text.startsWith('Слушаю')) { title.textContent = 'Вася слушает'; pill.className = 'island-pill'; }
      else if (event.text.startsWith('Диктовка ·')) { title.textContent = 'Диктую'; detail.textContent = '«Стоп диктовка» — завершить'; pill.className = 'island-pill'; }
    } else if (event.kind === 'info' || event.kind === 'answer') {
      title.textContent = event.kind === 'answer' ? 'Вася отвечает' : 'Готово'; detail.textContent = event.text;
      pill.className = 'island-pill done';
    } else if (event.kind === 'error') {
      title.textContent = 'Не получилось'; detail.textContent = event.text; pill.className = 'island-pill failed';
    }
  });
}
