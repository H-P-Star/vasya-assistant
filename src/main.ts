import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import './style.css';

type Activation = 'wake' | 'hotkey' | 'both';
type Settings = {
  wake_words: string[]; activation: Activation; hotkey: string; microphone: string;
  vosk_library: string; vosk_model: string; speech_mode: 'vosk' | 'whisper';
  whisper_executable: string; whisper_model: string; music_service: string; browser: string;
  voice_feedback: boolean; start_muted: boolean; start_in_tray: boolean; command_timeout: number;
  llm_provider: 'lm_studio' | 'ollama'; llm_url: string; llm_model: string;
  aliases: {phrase: string; action: string; payload: string}[];
};
type Snapshot = {settings: Settings; muted: boolean; busy: boolean; capture_open: boolean;
  data_directory: string; errors: string[]; status: string; last_error: string | null};
type AssistantEvent = {kind: string; text: string; level: number};

const native = isTauri();
let settings: Settings = {
  wake_words: ['вась', 'вася'], activation: 'both', hotkey: 'Ctrl+Alt+Space', microphone: '',
  vosk_library: '', vosk_model: '', speech_mode: 'vosk', whisper_executable: '', whisper_model: '',
  music_service: 'yandex', browser: 'edge', voice_feedback: true, start_muted: false,
  start_in_tray: false, command_timeout: 10, llm_provider: 'lm_studio', llm_url: 'http://198.18.0.1:1234',
  llm_model: 'google/gemma-3-4b', aliases: [],
};
let muted = false;
let busy = false;
let captureOpen = false;
let status = native ? 'Подключаюсь к помощнику…' : 'Предпросмотр интерфейса';
let page: 'home' | 'settings' | 'help' = 'home';
let error = '';
let result = '';
let heard = '';
let dataDirectory = '%LOCALAPPDATA%\\Vasya';
let devices: string[] = [];
let toastTimer: ReturnType<typeof setTimeout>;
const history: {time: string; kind: string; text: string}[] = [];
const serviceNames: Record<string, string> = {yandex: 'Яндекс Музыка', spotify: 'Spotify', youtube: 'YouTube'};
const paths: Record<string, string> = {
  mic: '<rect x="9" y="2" width="6" height="12" rx="3"/><path d="M5 10v2a7 7 0 0014 0v-2M12 19v3M8 22h8"/>',
  home: '<path d="M3 12l9-9 9 9M5 10v11h14V10M9 21v-7h6v7"/>',
  settings: '<path d="M4 7h16M4 17h16"/><circle cx="9" cy="7" r="3"/><circle cx="16" cy="17" r="3"/>',
  help: '<circle cx="12" cy="12" r="9"/><path d="M9.5 8a2.5 2.5 0 015 0c0 2-2.5 2-2.5 4M12 16v1"/>',
  arrow: '<path d="M5 12h14M13 6l6 6-6 6"/>',
  music: '<path d="M9 18V5l12-2v13M9 8l12-2"/><ellipse cx="6" cy="18" rx="3" ry="3"/><ellipse cx="18" cy="16" rx="3" ry="3"/>',
  shield: '<path d="M12 2L3 6v6c0 5 9 10 9 10s9-5 9-10V6zM8 12l3 3 5-6"/>',
  copy: '<rect x="8" y="8" width="13" height="13" rx="2"/><path d="M16 8V3H3v13h5"/>',
  key: '<rect x="2" y="5" width="20" height="14" rx="3"/><path d="M6 9h1m3 0h1m3 0h1m3 0h1M6 13h1m3 0h1m3 0h1m3 0h1M8 16h8"/>',
  pause: '<path d="M9 5v14M15 5v14"/>',
};
function icon(name: string, size = 20) {
  return `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths[name] || paths.mic}</svg>`;
}
function esc(value: string) { return value.replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]!)); }
function $(id: string) { return document.getElementById(id)!; }
function toast(message: string) {
  $('toast').textContent = message;
  $('toast').classList.add('visible');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => $('toast').classList.remove('visible'), 5000);
}
async function call<T>(command: string, args?: Record<string, unknown>): Promise<T | undefined> {
  if (!native) { toast('Голосовые функции доступны в Windows-приложении. Здесь — предпросмотр интерфейса.'); return; }
  try { return await invoke<T>(command, args); }
  catch (e) { error = String(e); showError(); toast(error); return; }
}
function log(kind: string, text: string) {
  history.unshift({kind, text, time: new Date().toLocaleTimeString('ru-RU', {hour:'2-digit', minute:'2-digit'})});
  history.splice(30);
  renderHistory();
}
function showError() {
  const area = document.getElementById('error');
  if (area) { area.hidden = !error; area.querySelector('span')!.textContent = error; }
}
function renderHistory() {
  const area = document.getElementById('history');
  if (!area) return;
  area.innerHTML = history.length ? history.slice(0, 6).map(e => `<li><time>${e.time}</time><span class="event-dot ${e.kind === 'error' ? 'danger' : ''}"></span><span>${esc(e.text)}</span></li>`).join('')
    : '<li class="empty-history">Здесь появятся распознанные запросы и результаты команд.</li>';
}
function updateStatus() {
  if (document.getElementById('status')) $('status').textContent = status;
  if (document.getElementById('mic-label')) $('mic-label').textContent = muted ? 'Включить микрофон' : 'Отключить микрофон';
  if (document.getElementById('mic-toggle')) {
    $('mic-toggle').setAttribute('aria-pressed', String(muted));
    $('mic-toggle').classList.toggle('is-muted', muted);
  }
  if (document.getElementById('listen')) ($('listen') as HTMLButtonElement).disabled = muted || busy;
  const indicator = document.getElementById('capture-state');
  if (indicator) indicator.textContent = muted ? 'Микрофон отключён' : captureOpen ? 'Микрофон используется' : 'Микрофон свободен';
  $('app').classList.toggle('muted', muted);
}

function navigate(next: typeof page) {
  if (page === next) return;
  page = next;
  render();
  window.scrollTo(0, 0);
}

function shell() {
  $('app').innerHTML = `
    <aside class="sidebar">
      <a class="brand" href="#home" aria-label="Вася, главная"><span class="brand-mark">${icon('mic', 23)}</span><span>вася<span class="brand-dot">.</span></span></a>
      <div class="sidebar-caption">ВАШ ГОЛОС. ВАШ КОМПЬЮТЕР.</div>
      <nav aria-label="Основная навигация">
        <button data-page="home">${icon('home')}<span>Помощник</span></button>
        <button data-page="settings">${icon('settings')}<span>Настройки</span></button>
        <button data-page="help">${icon('help')}<span>Команды и помощь</span></button>
      </nav>
      <div class="sidebar-bottom"><div class="local-note">${icon('shield')}<strong>Обработка на устройстве</strong><p>Речь распознаётся локально.<br>Ваш голос остаётся у вас.</p></div><span class="version">Вася для Windows <span>0.1.0</span></span></div>
    </aside>
    <main><div id="preview" ${native ? 'hidden' : ''}>Предпросмотр интерфейса · для работы с голосом запустите Windows-приложение</div><div id="content"></div></main>
    <div id="toast" role="status" aria-live="polite"></div>`;
  document.querySelectorAll<HTMLButtonElement>('[data-page]').forEach(button => button.onclick = () => {
    navigate(button.dataset.page as typeof page);
  });
  document.querySelector<HTMLAnchorElement>('.brand')!.onclick = e => { e.preventDefault(); navigate('home'); };
  render();
}

function render() {
  document.querySelectorAll('[data-page]').forEach(button => button.classList.toggle('active', (button as HTMLElement).dataset.page === page));
  const errorMarkup = '<div id="error" class="error-box" role="alert" hidden><span></span><button id="dismiss-error" aria-label="Скрыть ошибку">×</button></div>';
  const top = (title: string, description: string) => `<header class="page-header"><div><span class="eyebrow">ЛИЧНЫЙ ГОЛОСОВОЙ ПОМОЩНИК</span><h1>${title}</h1><p>${description}</p></div><span class="local-badge"><i></i> Локально</span></header>${errorMarkup}`;
  if (page === 'home') {
    $('content').innerHTML = top('Привет. Я Вася.', 'Помогу с музыкой, поиском и текстом. Просто позовите.') + `
      <section class="listen-card" aria-label="Состояние помощника">
        <div class="listen-top"><span class="section-label">НА СВЯЗИ</span><span id="capture-state" class="capture-label"></span></div>
        <div class="voice-orb">${icon('mic', 39)}</div>
        <h2 id="status" aria-live="polite"></h2>
        <p class="wake-hint">${settings.activation === 'hotkey' ? `Нажмите <kbd>${esc(settings.hotkey)}</kbd> и произнесите команду` : `Скажите <strong>«${esc(settings.wake_words.join('» или «'))}»</strong>${settings.activation === 'both' ? ` или нажмите <kbd>${esc(settings.hotkey)}</kbd>` : ''}`}</p>
        <div class="sound-meter" aria-hidden="true">${Array.from({length: 31}, (_, i) => `<i style="--height:${8 + (Math.sin(i * 1.9) + 1) * 12}px"></i>`).join('')}</div>
        <div class="listen-actions"><button id="listen" class="primary">${icon('mic', 17)} Слушать команду</button><button id="mic-toggle" class="secondary" aria-pressed="false">${icon('pause', 17)} <span id="mic-label"></span></button></div>
        <p class="quiet-note">Окно можно закрыть — помощник останется в трее.</p>
      </section>
      <div class="context-strip"><span>${icon('music', 17)} <strong>${serviceNames[settings.music_service]}</strong></span><span>${icon('shield', 17)} ${settings.speech_mode === 'vosk' ? 'Vosk' : 'Vosk + Whisper'} · на устройстве</span><button id="quick-settings">Настроить ${icon('arrow', 16)}</button></div>
      <section class="examples"><div class="section-heading"><h2>С чего начнём?</h2><span>Нажмите, чтобы попробовать</span></div><div class="example-grid">
        <button data-command="включи исполнителя Кино"><span class="example-icon peach">${icon('music')}</span><strong>Музыку под настроение</strong><span>«Вась, включи исполнителя Кино»</span></button>
        <button data-command="найди в браузере что такое IPv6"><span class="example-icon lilac">${icon('arrow')}</span><strong>Найти в интернете</strong><span>«Вась, найди, что такое IPv6»</span></button>
        <button data-command="напиши, что такое IPv6"><span class="example-icon mint">${icon('key')}</span><strong>Помочь с текстом</strong><span>«Вась, напиши, что такое IPv6»</span></button>
      </div></section>
      <section class="command-box"><label for="command">Или введите команду</label><form id="command-form"><input id="command" placeholder="Например: включи видео о космосе на ютубе" maxlength="8000" autocomplete="off"/><button class="icon-button" type="submit" aria-label="Выполнить команду">${icon('arrow')}</button></form><p>Диктовка и генерация из этой строки показывают текст в результате.</p></section>
      <section id="result-panel" class="result-panel" ${result ? '' : 'hidden'}><div class="section-heading"><h2>Результат</h2><div><button id="copy" class="text-button">${icon('copy', 15)} Копировать</button><button id="insert" class="text-button">Вставить через 3 секунды ${icon('arrow', 15)}</button></div></div><textarea id="result" aria-label="Результат распознавания или генерации" spellcheck="true"></textarea><p>Для вставки выберите целевое поле после нажатия кнопки.</p></section>
      <section class="history"><div class="section-heading"><h2>Последние действия</h2><button id="clear-history" class="text-button">Очистить</button></div><ul id="history"></ul></section>`;
    $('listen').onclick = () => { void call('activate'); };
    $('mic-toggle').onclick = async () => {
      if (!native) { await call('set_muted', {muted: !muted}); return; }
      const next = !muted;
      try {
        await invoke('set_muted', {muted: next});
        muted = next; updateStatus();
      } catch (e) { error = String(e); showError(); toast(error); }
    };
    $('quick-settings').onclick = () => { navigate('settings'); };
    $('command-form').onsubmit = async e => {
      e.preventDefault(); const input = $('command') as HTMLInputElement;
      if (!input.value.trim()) return;
      await call('run_text', {text: input.value});
    };
    document.querySelectorAll<HTMLButtonElement>('[data-command]').forEach(button => button.onclick = () => {
      const input = $('command') as HTMLInputElement;
      input.value = button.dataset.command!; input.focus();
    });
    ($('result') as HTMLTextAreaElement).value = result;
    $('result').oninput = () => { result = ($('result') as HTMLTextAreaElement).value; };
    $('copy').onclick = async () => { try { await navigator.clipboard.writeText(result); toast('Текст скопирован'); } catch { toast('Выделите текст и нажмите Ctrl+C'); } };
    $('insert').onclick = () => { toast('Выберите поле для вставки. Вставка через 3 секунды.'); void call('insert_result', {text: result}); };
    $('clear-history').onclick = () => { history.length = 0; renderHistory(); };
    renderHistory(); updateStatus();
  } else if (page === 'settings') {
    $('content').innerHTML = top('Настроим под вас.', 'Устройства, активация и локальные модели — всё в одном месте.') + settingsForm();
    fillSettings();
    $('settings-form').onsubmit = saveForm;
    $('refresh-mics').onclick = async () => {
      if (await refreshDevices()) { fillDevices(); toast('Список микрофонов обновлён'); }
    };
    $('login').onclick = () => { void call('open_music_login'); };
    ($('speech_mode') as HTMLSelectElement).onchange = updateWhisper;
    $('check-llm').onclick = async () => {
      const names = await call<string[]>('check_llm', {settings: readForm()});
      if (names) {
        $('llm-models').textContent = names.length ? `Доступные модели: ${names.join(', ')}` : 'Сервер доступен, моделей пока нет.';
        toast('Соединение с сервером установлено');
      }
    };
    ($('llm_provider') as HTMLSelectElement).onchange = () => {
      const ollama = ($('llm_provider') as HTMLSelectElement).value === 'ollama';
      ($('llm_url') as HTMLInputElement).value = ollama ? 'http://127.0.0.1:11434' : 'http://198.18.0.1:1234';
      ($('llm_model') as HTMLInputElement).value = ollama ? 'qwen2.5:3b' : 'google/gemma-3-4b';
      $('llm-models').textContent = '';
    };
    updateWhisper();
  } else {
    $('content').innerHTML = top('Скажите — и готово.', 'Справочник команд и короткая инструкция по запуску.') + `
      <section class="help-card"><h2>Первый запуск</h2><ol><li>Выполните <code>scripts/prepare-models.ps1</code> из проекта. Скрипт загрузит Vosk и русскую модель.</li><li>Выберите микрофон в настройках и сохраните их.</li><li>Для генерации запустите сервер LM Studio с моделью <code>google/gemma-3-4b</code>. Проверьте подключение в настройках; альтернативно можно выбрать Ollama.</li><li>Для музыки нажмите «Войти в сервис» в настройках. Музыкальный профиль браузера хранится отдельно.</li></ol></section>
      <section class="help-card"><h2>Команды</h2><table><thead><tr><th>Задача</th><th>После «Вась» или горячей клавиши</th></tr></thead><tbody>
      ${[
        ['Музыка', 'включи трек Перемен / исполнителя Кино / плейлист Работа'],
        ['Воспроизведение', 'пауза / продолжи / следующий трек'],
        ['Громкость', 'громче / тише / громкость 35 процентов'],
        ['Поиск', 'найди в браузере что такое IPv6'],
        ['Видео', 'включи видео о космосе на ютубе'],
        ['Разовая диктовка', 'диктуй Привет, Анна'],
        ['Непрерывная диктовка', 'начни диктовку → текст → стоп диктовка'],
        ['Генерация текста', 'напиши, что такое IPv6'],
        ['Микрофон', 'отключи микрофон (включение — кнопкой или через трей)'],
      ].map(([a,b]) => `<tr><td>${a}</td><td>${b}</td></tr>`).join('')}</tbody></table></section>
      <section class="help-card"><h2>Как это работает</h2><p>Можно произнести слово и команду одной фразой или позвать Васю, дождаться статуса «Слушаю команду» и продолжить. В режиме горячей клавиши устройство захвата открывается на время команды.</p><p>Для диктовки и генерации сначала поставьте курсор в нужное приложение, затем активируйте помощника. Если окно изменилось, текст останется в результате. Горячая клавиша также завершает непрерывную диктовку.</p><p>Vosk распознаёт слова активации обычным распознаванием речи. Это не отдельный детектор: возможны пропуски и ложные срабатывания. Whisper уточняет уже активированные фразы локально. Запросы к музыкальным сервисам, YouTube и поиску требуют интернета.</p><p>Пауза и продолжение адресуются текущей медиасессии Windows. На сайтах возможны ограничения автозапуска, входа и подписки. Помощник сообщает, если открыл только поиск.</p><p>Настройки и профиль: <code>${esc(dataDirectory)}</code>. Аудио не сохраняется постоянно; временный WAV для Whisper удаляется после обработки. История действий хранится только в памяти окна.</p></section>`;
  }
  showError();
  $('dismiss-error').onclick = () => { error = ''; showError(); };
}

function field(name: string, label: string, hint = '', type = 'text') {
  return `<label class="field">${label}<input id="${name}" name="${name}" type="${type}" ${type === 'number' ? 'min="3" max="60"' : ''}/>${hint ? `<small>${hint}</small>` : ''}</label>`;
}
function settingsForm() {
  return `<form id="settings-form"><section class="settings-card"><div class="card-title">${icon('mic')}<h2>Микрофон и активация</h2></div>
    <div class="field"><label for="microphone">Микрофон</label><div class="input-row"><select id="microphone"></select><button type="button" id="refresh-mics" class="secondary small">Обновить</button></div></div>
    <div class="form-grid"><label class="field">Способ активации<select id="activation"><option value="both">Слово и горячая клавиша</option><option value="wake">Только слово</option><option value="hotkey">Только горячая клавиша</option></select></label>${field('hotkey', 'Горячая клавиша', 'Например: Ctrl+Alt+Space')}</div>
    <div class="form-grid">${field('wake_words', 'Слова активации', 'Несколько вариантов через запятую')}${field('command_timeout', 'Ожидание команды, секунд', 'После слова или нажатия клавиши', 'number')}</div>
    <div class="check-grid"><label><input type="checkbox" id="voice_feedback"/> Голосовое подтверждение</label><label><input type="checkbox" id="start_muted"/> Запускать с выключенным микрофоном</label><label><input type="checkbox" id="start_in_tray"/> Открывать сразу в трее</label></div></section>
    <section class="settings-card"><div class="card-title">${icon('shield')}<h2>Распознавание речи</h2><span class="tag">Локально</span></div>
    <label class="field">Режим<select id="speech_mode"><option value="vosk">Vosk — быстрые команды и диктовка</option><option value="whisper">Vosk + Whisper — уточнение активированных фраз</option></select></label>
    ${field('vosk_library', 'Библиотека Vosk', 'Полный путь к libvosk.dll из Windows x64 архива Vosk')}${field('vosk_model', 'Папка модели Vosk', 'Распакованная русская модель, например vosk-model-small-ru-0.22')}
    <div id="whisper-fields">${field('whisper_executable', 'Путь к whisper-cli.exe')}${field('whisper_model', 'Файл модели Whisper', 'Локальный ggml-base.bin или другая многоязычная модель whisper.cpp')}</div>
    <p class="form-note">Vosk нужен в обоих режимах для активации и определения границ фраз. Модели не скачиваются без вашего действия.</p></section>
    <section class="settings-card"><div class="card-title">${icon('music')}<h2>Музыка и браузер</h2></div><div class="form-grid"><label class="field">Основной сервис<select id="music_service"><option value="yandex">Яндекс Музыка</option><option value="youtube">YouTube</option><option value="spotify">Spotify</option></select></label><label class="field">Браузер музыкального профиля<select id="browser"><option value="edge">Microsoft Edge</option><option value="chrome">Google Chrome</option></select></label></div>
    <button id="login" type="button" class="secondary">Войти в сервис ${icon('arrow', 16)}</button><p class="form-note">Сначала сохраните выбранный сервис. Музыка использует отдельный профиль браузера. Поиск и YouTube открываются в браузере по умолчанию.</p></section>
    <section class="settings-card"><div class="card-title">${icon('key')}<h2>Генерация текста</h2><span class="tag">Своя модель</span></div><label class="field">Провайдер<select id="llm_provider"><option value="lm_studio">LM Studio (совместимый API)</option><option value="ollama">Ollama</option></select></label><div class="form-grid">${field('llm_url', 'Адрес сервера', 'Без /v1 — путь добавляется автоматически')}${field('llm_model', 'Идентификатор модели')}</div><button id="check-llm" type="button" class="secondary">Проверить соединение ${icon('arrow', 16)}</button><p id="llm-models" class="form-note"></p><p class="form-note">Текст запросов «пиши/напиши» отправляется на указанный сервер. Обычные команды и аудио туда не отправляются.</p></section>
    <div class="save-row"><span>Настройки применятся после сохранения.</span><button type="submit" class="primary">Сохранить настройки ${icon('arrow', 17)}</button></div></form>`;
}
function fillDevices() {
  const select = $('microphone') as HTMLSelectElement;
  const previous = select.value || settings.microphone;
  const names = [...new Set([...devices, ...(previous ? [previous] : [])])];
  select.innerHTML = '<option value="">Микрофон Windows по умолчанию</option>' + names.map(name => `<option value="${esc(name)}">${esc(name)}</option>`).join('');
  select.value = previous;
}
function fillSettings() {
  for (const [key, value] of Object.entries(settings)) {
    const element = document.getElementById(key) as HTMLInputElement | HTMLSelectElement | null;
    if (!element) continue;
    if (typeof value === 'boolean') (element as HTMLInputElement).checked = value;
    else element.value = Array.isArray(value) ? value.join(', ') : String(value);
  }
  fillDevices();
}
function updateWhisper() { $('whisper-fields').hidden = ($('speech_mode') as HTMLSelectElement).value !== 'whisper'; }
function readForm(): Settings {
  const next = structuredClone(settings);
  for (const [key, value] of Object.entries(settings)) {
    const element = document.getElementById(key) as HTMLInputElement | HTMLSelectElement | null;
    if (!element) continue;
    (next as Record<string, unknown>)[key] = typeof value === 'boolean' ? (element as HTMLInputElement).checked
      : typeof value === 'number' ? Number(element.value) : key === 'wake_words'
      ? element.value.split(',').map(w => w.trim().toLowerCase()).filter(Boolean) : element.value.trim();
  }
  return next;
}
async function saveForm(event: Event) {
  event.preventDefault();
  const next = readForm();
  if (!next.wake_words.length) { toast('Введите хотя бы одно слово активации'); return; }
  if (!native) { toast('В предпросмотре настройки не сохраняются. Запустите Windows-приложение.'); return; }
  try { await invoke('save_settings', {settings: next}); settings = next; error = ''; showError(); toast('Настройки сохранены. Перезапускаю распознавание.'); }
  catch (e) { error = String(e); showError(); }
}
async function refreshDevices(): Promise<boolean> {
  const names = await call<string[]>('list_microphones');
  if (!names) return false;
  devices = names;
  return true;
}

shell();
async function connect() {
  if (!native) return;
  await listen<AssistantEvent>('assistant', ({payload: event}) => {
    if (event.kind === 'level') {
      document.querySelectorAll<HTMLElement>('.sound-meter i').forEach((bar, i) => {
        bar.style.height = `${6 + event.level * 45 * (0.4 + Math.abs(Math.sin(i * 1.9)) * 0.6)}px`;
      });
      return;
    }
    if (event.kind === 'status') { status = event.text; updateStatus(); }
    if (event.kind === 'microphone') { muted = event.text === 'muted'; updateStatus(); }
    if (event.kind === 'heard') { heard = event.text; log('heard', `Вы: ${heard}`); }
    if (event.kind === 'info') { log('info', event.text); toast(event.text); }
    if (event.kind === 'error') { error = event.text; showError(); log('error', error); }
    if (event.kind === 'result') {
      result = event.text;
      const panel = document.getElementById('result-panel');
      if (panel) { panel.hidden = false; ($('result') as HTMLTextAreaElement).value = result; }
      else toast('Текст готов. Результат доступен на вкладке «Помощник».');
    }
  });
  const initial = await call<Snapshot>('get_state');
  if (initial) {
    settings = initial.settings; muted = initial.muted; busy = initial.busy; captureOpen = initial.capture_open;
    status = initial.status; dataDirectory = initial.data_directory;
    error = [...initial.errors, ...(initial.last_error ? [initial.last_error] : [])].join('\n');
    render();
  }
  await refreshDevices();
  setInterval(async () => {
    try {
      const state = await invoke<Snapshot>('get_state');
      muted = state.muted; busy = state.busy; captureOpen = state.capture_open; status = state.status;
      updateStatus();
    } catch { /* A transient IPC failure is reported on the next user action. */ }
  }, 1000);
}
void connect();
