import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import './style.css';
import { startIsland } from './island';

import type {Settings,Alias,Builtin,BuiltinOverride} from './types';
import {calibrationUI} from './calibration-ui';
type Snapshot = {speech_ready: boolean; settings: Settings; muted: boolean; busy: boolean; capture_open: boolean;
  data_directory: string; errors: string[]; status: string; last_error: string | null; last_result: string; llm_key_set: boolean};
type AssistantEvent = {kind: string; text: string; level: number};

const native = isTauri();
let settings: Settings = {
  onboarding_complete: false, wake_words: ['вась', 'вася'], activation: 'both', hotkey: 'Ctrl+Alt+Space', microphone: '',
  vosk_library: '', vosk_model: '', speech_mode: 'whisper', whisper_executable: '', whisper_model: '',
  speech_language:'auto',speech_hint:'',speech_gain:1,noise_floor:0.003,wake_aliases:[],speech_corrections:[],
  tts_engine:'windows',tts_voice:'',tts_volume:80,tts_rate:0,piper_executable:'',piper_model:'',piper_speaker:0,voice_presets:[],
  music_service: 'yandex', browser: 'edge', music_mode: 'desktop', yandex_app_path: '', spotify_app_path: '',
  island_enabled: true, llm_fallback: true, voice_feedback: true, start_muted: false,
  start_in_tray: false, command_timeout: 10, llm_provider: 'lm_studio', llm_url: 'http://198.18.0.1:1234',
  llm_model: 'google/gemma-3-4b',llm_structured:true, aliases: [],builtin_overrides:[],
};
let connected=!native;
let speechReady = false;
let muted = false;
let busy = false;
let captureOpen = false;
let status = native ? 'Подключаюсь к помощнику…' : 'Предпросмотр интерфейса';
let disposePage=()=>{};
let page: 'home' | 'settings' | 'commands' | 'calibration' | 'help' = 'home';
let aliasDraft: Alias[] | undefined;
let builtinDraft: BuiltinOverride[] | undefined;
let builtins: Builtin[]=[];
let voices: {id:string;name:string}[]=[];
let keySet=false;
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
  disposePage();disposePage=()=>{};
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
        <button data-page="calibration">${icon('mic')}<span>Калибровка речи</span></button>
        <button data-page="commands">${icon('key')}<span>Мои команды</span></button>
        <button data-page="help">${icon('help')}<span>Команды и помощь</span></button>
      </nav>
      <div class="sidebar-bottom"><div class="local-note">${icon('shield')}<strong>Обработка на устройстве</strong><p>Речь распознаётся локально.<br>Ваш голос остаётся у вас.</p></div><span class="version">Вася для Windows <span>0.4.0</span></span></div>
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
  if(!connected){$('content').innerHTML=top('Подключаюсь к Васе…','Загружаю сохранённые настройки.');showError();$('dismiss-error').onclick=()=>{error='';showError();};return;}
  if (page === 'home' && !settings.onboarding_complete) {
    $('content').innerHTML = top('Давайте познакомимся.', 'Два шага — и можно говорить с Васей.') + `
      <section class="settings-card"><h2>1. Выберите микрофон</h2>
      <p>Компоненты русского распознавания ${speechReady ? 'установлены вместе с приложением' : 'пока не найдены — переустановите полную версию Васи'}. Интернет для распознавания не нужен.</p>
      <label class="field">Микрофон<select id="microphone"></select></label>
      <button id="test-mic" class="secondary">Проверить микрофон · 2 секунды</button><p id="mic-test" role="status">Нажмите кнопку и произнесите несколько слов. Команды во время проверки не выполняются.</p>
      <h2>2. Начните пользоваться</h2><p>Скажите «Вася, громче» или нажмите <kbd>${esc(settings.hotkey)}</kbd> и скажите «найди погоду».</p>
      <p>Для музыки выберите сервис в настройках и войдите в аккаунт. Модель для генерации и ответов через LM Studio или Ollama можно подключить позже — обычные команды работают без них.</p>
      <button id="finish-setup" class="primary" ${native && speechReady ? '' : 'disabled'}>Начать пользоваться</button></section>`;
    fillDevices();
    $('test-mic').onclick = async () => {
      if (!native) { toast('Проверка доступна в установленном приложении'); return; }
      const button = $('test-mic') as HTMLButtonElement;
      const output = $('mic-test');
      button.disabled = true; output.textContent = 'Микрофон включён на 2 секунды. Говорите…';
      try {
        const level = await invoke<number>('test_microphone', {microphone: ($('microphone') as HTMLSelectElement).value});
        output.textContent = level > 0.005 ? 'Звук получен. Микрофон работает.' : 'Звук очень тихий. Проверьте устройство и громкость микрофона.';
      } catch (e) { output.textContent = String(e); }
      finally { button.disabled = false; }
    };
    $('finish-setup').onclick = async () => {
      try {
        await invoke('finish_setup', {microphone: ($('microphone') as HTMLSelectElement).value});
        const state = await invoke<Snapshot>('get_state');
        settings = state.settings; muted = state.muted; error = ''; render();
      } catch (e) { error = String(e); showError(); }
    };
    showError(); $('dismiss-error').onclick = () => { error = ''; showError(); };
    return;
  }
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
      <section id="result-panel" class="result-panel" ${result ? '' : 'hidden'}><div class="section-heading"><h2>Результат</h2><div><button id="copy" class="text-button">${icon('copy', 15)} Копировать</button><button id="show-last-result" class="text-button">Открыть в панели</button><button id="insert" class="text-button">Вставить через 3 секунды ${icon('arrow', 15)}</button></div></div><textarea id="result" aria-label="Результат распознавания или генерации" spellcheck="true"></textarea><p>Для вставки выберите целевое поле после нажатия кнопки.</p></section>
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
    $('show-last-result').onclick=()=>{void call('show_last_result');};
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
    document.querySelectorAll<HTMLButtonElement>('[data-resource]').forEach(button=>button.onclick=()=>{void call('open_resource',{kind:button.dataset.resource});});
    $('get-api-key').onclick=()=>{void call('open_resource',{kind:($('llm_provider') as HTMLSelectElement).value});};
    $('restart-music').onclick=()=>{void call('restart_music_app');};
    $('open-calibration').onclick=()=>navigate('calibration');
    bindVoiceForm();
    $('api-key-status').textContent=keySet?'Ключ для выбранного провайдера сохранён.':'Ключ не сохранён. Для локальных серверов обычно не требуется.';
    const storeKey=async(value:string)=>{
      if(!native){toast('Ключ хранится в Windows-приложении.');return;}
      try{await invoke('save_api_key',{provider:($('llm_provider') as HTMLSelectElement).value,key:value});keySet=!!value;($('api-key') as HTMLInputElement).value='';$('api-key-status').textContent=keySet?'Ключ защищён Windows и сохранён.':'Ключ удалён.';}catch(e){error=String(e);showError();}
    };
    $('save-api-key').onclick=()=>{const value=($('api-key') as HTMLInputElement).value.trim();if(!value){toast('Введите API-ключ. Для удаления используйте отдельную кнопку.');return;}void storeKey(value);};
    $('delete-api-key').onclick=()=>{void storeKey('');};
    ($('speech_mode') as HTMLSelectElement).onchange = updateWhisper;
    $('check-llm').onclick = async () => {
      const names = await call<string[]>('check_llm', {settings: readForm()});
      if (names) {
        $('llm-models').textContent = names.length ? `Доступно ${names.length} моделей. Начало списка: ${names.slice(0,12).join(', ')}` : 'Сервер доступен, моделей пока нет.';
        const list=document.getElementById('model-options');if(list)list.innerHTML=names.map(n=>`<option value="${esc(n)}"></option>`).join('');
        toast('Соединение с сервером установлено');
      }
    };
    ($('llm_provider') as HTMLSelectElement).onchange = () => {
      const provider=($('llm_provider') as HTMLSelectElement).value;
      const defaults:Record<string,[string,string]>={lm_studio:['http://198.18.0.1:1234','google/gemma-3-4b'],ollama:['http://127.0.0.1:11434','qwen2.5:3b'],open_router:['https://openrouter.ai/api/v1','openai/gpt-4.1-mini'],open_ai:['https://api.openai.com/v1','gpt-4.1-mini'],compatible:['https://example.com/v1','model-id']};
      ($('llm_url') as HTMLInputElement).value=defaults[provider][0];($('llm_model') as HTMLInputElement).value=defaults[provider][1];
      ($('api-key') as HTMLInputElement).value='';$('api-key-status').textContent='Ключи провайдеров хранятся отдельно. Сохраните ключ, если он ещё не добавлен.';
      $('llm-models').textContent = '';
    };
    updateWhisper();
  } else if (page === 'calibration') {
    $('content').innerHTML=top('Чтобы понимать с полуслова.','Микрофон, обращения и ваши названия.')+'<div id="calibration-root"></div>';
    disposePage=calibrationUI($('calibration-root'),settings,native,async next=>{await invoke('save_settings',{settings:next});settings=next;});
  } else if (page === 'commands') {
    $('content').innerHTML = top('Ваши слова. Ваши команды.', 'Меняйте привычные фразы и добавляйте свои действия.') + commandEditor();
    bindCommandEditor();
  } else {
    $('content').innerHTML = top('Скажите — и готово.', 'Справочник команд и короткая инструкция по запуску.') + `
      <section class="help-card"><h2>Первый запуск</h2><ol><li>Установите Васю из полного установщика .exe в GitHub Releases. Vosk и русская модель уже включены.</li><li>При первом запуске выберите и проверьте микрофон, затем нажмите «Начать пользоваться».</li><li>Для генерации запустите сервер LM Studio с моделью <code>google/gemma-3-4b</code>. Проверьте подключение в настройках; альтернативно можно выбрать Ollama.</li><li>Для музыки выберите сервис и нажмите «Открыть музыкальный сервис» в настройках. По умолчанию используется установленное приложение.</li></ol></section>
      <section class="help-card"><h2>Команды</h2><table><thead><tr><th>Задача</th><th>После «Вась» или горячей клавиши</th></tr></thead><tbody>
      ${[
        ['Музыка', 'включи трек Перемен / исполнителя Кино / плейлист Работа'],
        ['Музыкальное приложение', 'открой Яндекс Музыку / открой Spotify'],
        ['Свои команды', 'Добавьте фразы и действия во вкладке «Мои команды»'],
        ['Вопрос модели', 'расскажи, что такое IPv6 (при включённом обращении к модели)'],
        ['Воспроизведение', 'пауза / продолжи / следующий трек'],
        ['Громкость', 'громче / тише / громкость 35 процентов'],
        ['Поиск', 'найди в браузере что такое IPv6'],
        ['Видео', 'включи видео о космосе на ютубе'],
        ['Разовая диктовка', 'диктуй Привет, Анна'],
        ['Непрерывная диктовка', 'начни диктовку → текст → стоп диктовка'],
        ['Генерация текста', 'напиши, что такое IPv6'],
        ['Микрофон', 'отключи микрофон (включение — кнопкой или через трей)'],
      ].map(([a,b]) => `<tr><td>${a}</td><td>${b}</td></tr>`).join('')}</tbody></table></section>
      <section class="help-card"><h2>Как это работает</h2><p>Можно произнести слово и команду одной фразой или позвать Васю, дождаться статуса «Слушаю команду» и продолжить. В режиме горячей клавиши устройство захвата открывается на время команды.</p><p>Для диктовки и генерации сначала поставьте курсор в нужное приложение, затем активируйте помощника. Если окно изменилось, текст останется в результате. Горячая клавиша также завершает непрерывную диктовку.</p><p>Для обращения используется дополнительный распознаватель Vosk с ограниченной грамматикой. Возможны пропуски и ложные срабатывания; проверить обращение можно в калибровке. Whisper уточняет активированные фразы локально, включая английские названия. Запросы к музыкальным сервисам, YouTube и поиску требуют интернета.</p><p>Пауза и продолжение адресуются текущей медиасессии Windows. В музыкальных приложениях и на сайтах возможны ограничения автозапуска, входа и подписки. Помощник сообщает, если открыл только поиск.</p><p>Настройки и профиль: <code>${esc(dataDirectory)}</code>. Аудио не сохраняется постоянно; временный WAV для Whisper удаляется после обработки. История действий хранится только в памяти окна.</p></section>`;
  }
  showError();
  $('dismiss-error').onclick = () => { error = ''; showError(); };
}

const commandActions: Record<string, string> = {
  open_yandex:'Открыть Яндекс Музыку',open_spotify:'Открыть Spotify',cancel:'Отмена',
  music: 'Включить трек', music_artist: 'Включить исполнителя', music_playlist: 'Включить плейлист',
  search: 'Поиск в браузере', youtube: 'Видео YouTube', generate: 'Сгенерировать текст',
  insert: 'Вставить текст', answer: 'Ответить фразой', launch_app: 'Открыть программу (.exe)',
  pause: 'Пауза', resume: 'Продолжить музыку', next: 'Следующий трек', volume: 'Громкость 0–100',
  volume_up: 'Громче', volume_down: 'Тише', dictation_start: 'Начать диктовку',
  dictation_stop: 'Завершить диктовку', mute: 'Отключить микрофон', disabled: 'Не выполнять эту фразу',
};
function commandEditor() {
  aliasDraft ??= structuredClone(settings.aliases);
  builtinDraft ??= structuredClone(settings.builtin_overrides);
  return `<section class="settings-card command-intro"><h2>Как добавить команду</h2>
    <p>Укажите фразу без слова «Вася», выберите действие и его содержимое. Своя фраза имеет приоритет над встроенной.</p>
    <p>Для переменной части используйте <code>{query}</code> в нужном месте фразы. Например: «поищи {query}» → поиск. В содержимом можно написать «{query} официальный сайт» или оставить его пустым для передачи запроса целиком.</p>
    <p>Чтобы изменить встроенную команду, добавьте ту же фразу со своим действием. Действие «Не выполнять эту фразу» блокирует её. Удаление правила возвращает встроенное поведение.</p>
    <div class="command-toolbar"><button id="add-command" class="primary">+ Добавить команду</button><button id="add-template" class="secondary">Пример с запросом</button></div></section>
    <form id="commands-form"><section class="settings-card"><h2>Встроенные команды</h2><p class="form-note">Изменяйте фразы, действие и содержимое. «Восстановить» возвращает исходное правило. Стандартное «стоп диктовка» остаётся доступным для выхода из активной диктовки.</p><div id="builtin-rows"></div></section><h2 class="custom-heading">Свои команды</h2><div id="command-rows"></div><div class="save-row"><span id="command-save-note">Изменения применятся после сохранения.</span><button class="primary" type="submit">Сохранить команды ${icon('arrow',17)}</button></div></form>`;
}
function renderBuiltins() {
  const area=document.getElementById('builtin-rows');if(!area)return;
  if(!builtins.length){area.innerHTML='<p class="form-note">Каталог встроенных команд загружается из Windows-приложения.</p>';return;}
  area.innerHTML=builtins.map(b=>{
    const rule=builtinDraft?.find(r=>r.id===b.id)||{id:b.id,enabled:true,phrases:b.phrases,action:b.action,payload:b.payload};
    return `<details class="builtin-rule" data-builtin="${b.id}"><summary>${esc(b.title)} <span>${builtinDraft?.some(r=>r.id===b.id)?'Изменено':'По умолчанию'}</span></summary>
    <label class="rule-enabled"><input type="checkbox" data-bkey="enabled" ${rule.enabled?'checked':''}/> Команда включена</label>
    <label class="field">Фразы · по одной в строке<textarea data-bkey="phrases" rows="${Math.min(5,rule.phrases.length)}">${esc(rule.phrases.join('\n'))}</textarea><small>Переменная часть — {query}; можно добавить слова после неё.</small></label>
    <div class="form-grid"><label class="field">Действие<select data-bkey="action">${Object.entries(commandActions).map(([id,label])=>`<option value="${id}" ${rule.action===id?'selected':''}>${label}</option>`).join('')}</select></label><label class="field">Содержимое<input data-bkey="payload" value="${esc(rule.payload)}" placeholder="Обычно пусто или {query}"/></label></div><button type="button" class="text-button builtin-reset">Восстановить встроенную команду</button></details>`;
  }).join('');
  area.querySelectorAll<HTMLElement>('[data-builtin]').forEach(section=>{
    const id=section.dataset.builtin!;
    const saveDraft=()=>{
      const value=(key:string)=>(section.querySelector(`[data-bkey="${key}"]`) as HTMLInputElement).value;
      const rule:BuiltinOverride={id,enabled:(section.querySelector('[data-bkey="enabled"]') as HTMLInputElement).checked,phrases:value('phrases').split('\n').map(s=>s.trim()).filter(Boolean),action:value('action'),payload:value('payload')};
      builtinDraft=[...builtinDraft!.filter(r=>r.id!==id),rule];$('command-save-note').textContent='Есть несохранённые изменения.';
      section.querySelector('summary span')!.textContent='Изменено';
    };
    section.querySelectorAll<HTMLInputElement>('[data-bkey]').forEach(input=>{input.oninput=saveDraft;input.onchange=saveDraft;});
    section.querySelector<HTMLButtonElement>('.builtin-reset')!.onclick=()=>{builtinDraft=builtinDraft!.filter(r=>r.id!==id);renderBuiltins();$('command-save-note').textContent='Восстановление применится после сохранения.';};
  });
}
function renderCommandRows() {
  const rows = aliasDraft!;
  $('command-rows').innerHTML = rows.length ? rows.map((row, index) => `<section class="settings-card command-rule" data-rule="${index}">
    <div class="section-heading"><label class="rule-enabled"><input type="checkbox" data-key="enabled" ${row.enabled ? 'checked' : ''}/> Команда ${index + 1}</label><button type="button" class="text-button delete-rule" aria-label="Удалить команду ${index + 1}">Удалить</button></div>
    <div class="form-grid"><label class="field">Когда я говорю<input data-key="phrase" value="${esc(row.phrase)}" placeholder="Например: мой плейлист" maxlength="512" required/></label>
    <label class="field">Вася должен<select data-key="action">${Object.entries(commandActions).map(([key,label]) => `<option value="${key}" ${row.action === key ? 'selected' : ''}>${label}</option>`).join('')}</select></label></div>
    <label class="field rule-payload">Содержимое действия<textarea data-key="payload" maxlength="16000" rows="2" placeholder="Название, запрос, текст или полный путь к .exe">${esc(row.payload)}</textarea><small>Для паузы, переключения, диктовки и отключения микрофона содержимое не требуется.</small></label>
    </section>`).join('') : '<section class="settings-card command-empty">Пока нет своих команд. Встроенные команды уже работают.</section>';
  document.querySelectorAll<HTMLElement>('[data-rule]').forEach(section => {
    const index = Number(section.dataset.rule);
    section.querySelectorAll<HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement>('[data-key]').forEach(input => {
      const update = () => {
        const key = input.dataset.key!;
        (rows[index] as Record<string, unknown>)[key] = key === 'enabled' ? (input as HTMLInputElement).checked : input.value;
        $('command-save-note').textContent = 'Есть несохранённые изменения.';
      };
      input.oninput = update; input.onchange = update;
    });
    section.querySelector<HTMLButtonElement>('.delete-rule')!.onclick = () => { rows.splice(index,1); renderCommandRows(); };
  });
}
function bindCommandEditor() {
  renderCommandRows();
  renderBuiltins();
  const add = (template: boolean) => {
    aliasDraft!.push({enabled:true, phrase: template ? 'поищи {query}' : '', action: template ? 'search' : 'music_playlist', payload:''});
    renderCommandRows();
    document.querySelector<HTMLInputElement>('[data-rule]:last-child [data-key="phrase"]')?.focus();
  };
  $('add-command').onclick = () => add(false);
  $('add-template').onclick = () => add(true);
  $('commands-form').onsubmit = async event => {
    event.preventDefault();
    if (!native) { toast('Команды сохраняются в Windows-приложении.'); return; }
    const next = {...settings, aliases: structuredClone(aliasDraft!).map(a => ({...a, phrase:a.phrase.trim()})),builtin_overrides:structuredClone(builtinDraft!)};
    try {
      await invoke('save_settings', {settings:next}); settings = next; error = ''; showError();
      $('command-save-note').textContent = 'Команды сохранены.'; toast('Команды сохранены и готовы к использованию.');
    } catch (e) { error = String(e); showError(); }
  };
}

function field(name: string, label: string, hint = '', type = 'text') {
  return `<label class="field">${label}<input id="${name}" name="${name}" type="${type}" ${type === 'number' ? 'min="3" max="60"' : ''}/>${hint ? `<small>${hint}</small>` : ''}</label>`;
}
function voiceForm(){
  const available=[...voices];if(settings.tts_voice&&!available.some(v=>v.id===settings.tts_voice))available.push({id:settings.tts_voice,name:'Сохранённый голос'});
  return `<section class="settings-card"><div class="card-title">${icon('mic')}<h2>Голос Васи</h2></div><label class="field">Движок<select id="tts_engine"><option value="windows">Голоса Windows · без загрузок</option><option value="piper">Piper · скачанный нейросетевой голос</option></select></label><div id="windows-voice"><label class="field">Установленный голос<select id="tts_voice"><option value="">Голос Windows по умолчанию</option>${available.map(v=>`<option value="${esc(v.id)}">${esc(v.name)}</option>`).join('')}</select></label></div>
  <div id="piper-voice">${field('piper_executable','Путь к piper.exe','Распакуйте весь архив Windows x64 вместе с DLL')}${field('piper_model','Модель голоса .onnx','Рядом должен лежать файл с тем же именем .onnx.json')}<label class="field">Номер диктора<input id="piper_speaker" type="number" min="0" max="10000"/><small>Обычно 0. Другие номера используются только многоголосыми моделями.</small></label><p class="form-note">Можно переключать мужские и женские модели, например русские Dmitri и Irina. Голоса и их конфигурации скачиваются отдельно. Для готового piper.exe используйте совместимые голоса v1.0.0: скачайте .onnx и .onnx.json с одинаковым именем.</p><div class="command-toolbar"><button type="button" class="text-button" data-resource="piper">Скачать piper.exe</button><button type="button" class="text-button" data-resource="voices">Мужские и женские голоса RU</button><button type="button" class="text-button" data-resource="docs">Инструкция</button></div></div>
  <div class="form-grid"><label class="field">Громкость голоса <output id="voice-volume-value"></output><input id="tts_volume" type="range" min="0" max="100" step="1"/><small>Отдельно от громкости музыки и Windows.</small></label><label class="field">Скорость <output id="voice-rate-value"></output><input id="tts_rate" type="range" min="-10" max="10" step="1"/></label></div>
  <div class="form-grid"><label class="field">Пресет<select id="voice-preset"><option value="">Выберите пресет</option><option value="calm">Спокойный · медленнее</option><option value="normal">Обычный</option><option value="bright">Энергичный · быстрее</option>${settings.voice_presets.map((p,i)=>`<option value="custom-${i}">${esc(p.name)}</option>`).join('')}</select></label><label class="field">Имя своего пресета<input id="voice-preset-name" placeholder="Например: Вася — Дмитрий"/></label></div><div class="command-toolbar"><button id="test-voice" class="secondary" type="button">Послушать голос</button><button id="save-voice-preset" class="text-button" type="button">Добавить свой пресет</button><button id="delete-voice-preset" class="text-button" type="button">Удалить выбранный</button></div></section>`;
}
function bindVoiceForm(){
 const update=()=>{const piper=($('tts_engine') as HTMLSelectElement).value==='piper';$('piper-voice').hidden=!piper;$('windows-voice').hidden=piper;$('voice-volume-value').textContent=($('tts_volume') as HTMLInputElement).value+'%';$('voice-rate-value').textContent=($('tts_rate') as HTMLInputElement).value;};
 $('tts_engine').onchange=update;$('tts_volume').oninput=update;$('tts_rate').oninput=update;
 $('test-voice').onclick=()=>{void call('test_voice',{settings:readForm()});};
 const refreshPresets=()=>{const select=$('voice-preset') as HTMLSelectElement;select.querySelectorAll('option[value^="custom-"]').forEach(o=>o.remove());settings.voice_presets.forEach((p,i)=>select.add(new Option(p.name,'custom-'+i)));};
 $('voice-preset').onchange=()=>{const value=($('voice-preset') as HTMLSelectElement).value;if(!value)return;
  if(!value.startsWith('custom-')){($('tts_rate') as HTMLInputElement).value=String({calm:-2,normal:0,bright:2}[value as 'calm'|'normal'|'bright']);update();return;}
  const preset=settings.voice_presets[Number(value.slice(7))];if(!preset)return;
  for(const [id,v]of Object.entries({tts_engine:preset.engine,tts_voice:preset.voice,piper_model:preset.model,piper_speaker:preset.speaker,tts_rate:preset.rate})){const input=$(id) as HTMLInputElement;input.value=String(v);}update();
 };
 $('save-voice-preset').onclick=()=>{const name=($('voice-preset-name') as HTMLInputElement).value.trim();if(!name){toast('Введите имя пресета.');return;}const next=readForm();settings.voice_presets=[...settings.voice_presets.filter(p=>p.name!==name),{name,engine:next.tts_engine,voice:next.tts_voice,model:next.piper_model,speaker:next.piper_speaker,rate:next.tts_rate}];refreshPresets();toast('Пресет добавлен. Нажмите «Сохранить настройки».');};
 $('delete-voice-preset').onclick=()=>{const value=($('voice-preset') as HTMLSelectElement).value;if(value.startsWith('custom-')){settings.voice_presets.splice(Number(value.slice(7)),1);refreshPresets();toast('Пресет удалён. Сохраните настройки.');}};
 update();
 const model=$('llm_model') as HTMLInputElement;model.setAttribute('list','model-options');model.insertAdjacentHTML('afterend','<datalist id="model-options"></datalist>');
}
function settingsForm() {
  return `<form id="settings-form"><section class="settings-card"><div class="card-title">${icon('mic')}<h2>Микрофон и активация</h2></div>
    <div class="field"><label for="microphone">Микрофон</label><div class="input-row"><select id="microphone"></select><button type="button" id="refresh-mics" class="secondary small">Обновить</button></div></div>
    <div class="form-grid"><label class="field">Способ активации<select id="activation"><option value="both">Слово и горячая клавиша</option><option value="wake">Только слово</option><option value="hotkey">Только горячая клавиша</option></select></label>${field('hotkey', 'Горячая клавиша', 'Например: Ctrl+Alt+Space')}</div>
    <div class="form-grid">${field('wake_words', 'Слова активации', 'Несколько вариантов через запятую')}${field('command_timeout', 'Ожидание команды, секунд', 'После слова или нажатия клавиши', 'number')}</div>
    <div class="check-grid"><label><input type="checkbox" id="voice_feedback"/> Голосовое подтверждение</label><label><input type="checkbox" id="island_enabled"/> Показывать панель активации поверх окон</label><label><input type="checkbox" id="start_muted"/> Запускать с выключенным микрофоном</label><label><input type="checkbox" id="start_in_tray"/> Открывать сразу в трее</label></div></section>
    <section class="settings-card"><div class="card-title">${icon('shield')}<h2>Распознавание речи</h2><span class="tag">Локально</span></div>
    <label class="field">Режим<select id="speech_mode"><option value="vosk">Vosk — быстрые команды и диктовка</option><option value="whisper">Vosk + Whisper — уточнение активированных фраз</option></select></label>
    <label class="field">Язык команд<select id="speech_language"><option value="auto">Автоматически · русский и английский</option><option value="ru">Русский</option><option value="en">Английский</option></select></label>
    ${field('speech_hint','Подсказка словаря','Например: Linkin Park, Spotify, IPv6. Используется Whisper.')}
    <button type="button" class="secondary" id="open-calibration">Калибровать распознавание ${icon('arrow',16)}</button>
    <p class="form-note">Vosk и многоязычный Whisper включены в установщик. Для английских названий используйте Vosk + Whisper. Калибровка настраивает усиление, обращения и словарь исправлений.</p>
    <details><summary>Дополнительно: своя модель распознавания</summary>
    ${field('vosk_library', 'Библиотека Vosk', 'Оставьте пустым для встроенной библиотеки')}${field('vosk_model', 'Папка модели Vosk', 'Оставьте пустым для встроенной русской модели')}
    </details>
    <div id="whisper-fields"><details><summary>Дополнительно: свой Whisper</summary>${field('whisper_executable', 'Путь к whisper-cli.exe','Пусто — встроенный Whisper')}${field('whisper_model', 'Файл модели Whisper', 'Пусто — встроенная многоязычная base; можно указать другую модель whisper.cpp')}</details></div>
    <p class="form-note">Вызов распознаётся Vosk, затем Whisper уточняет активированную фразу с учётом выбранного языка.</p></section>
    ${voiceForm()}
    <section class="settings-card"><div class="card-title">${icon('music')}<h2>Музыка и браузер</h2></div><div class="form-grid"><label class="field">Основной сервис<select id="music_service"><option value="yandex">Яндекс Музыка</option><option value="youtube">YouTube</option><option value="spotify">Spotify</option></select></label><label class="field">Браузер музыкального профиля<select id="browser"><option value="edge">Microsoft Edge</option><option value="chrome">Google Chrome</option></select></label></div>
    <label class="field">Где запускать музыку<select id="music_mode"><option value="desktop">В установленном приложении</option><option value="browser">В отдельном музыкальном профиле браузера</option></select></label>
    <details><summary>Пути к музыкальным приложениям</summary>${field('yandex_app_path','Приложение Яндекс Музыки','Пусто — найти автоматически в LocalAppData\\Programs\\YandexMusic')}${field('spotify_app_path','Приложение Spotify','Пусто — найти автоматически или использовать протокол Spotify')}</details>
    <div class="command-toolbar"><button id="login" type="button" class="secondary">Подключить музыкальный сервис ${icon('arrow',16)}</button><button id="restart-music" type="button" class="secondary">Перезапустить и подключить Яндекс Музыку</button></div><p class="form-note">Сначала сохраните выбранный сервис и путь. Перезапуск остановит текущую музыку и включит управление официальным клиентом. В дальнейшем Вася восстановит подключение автоматически. Пример: «включи трек Перемен исполнителя Кино». В Spotify пока требуется выбрать результат вручную.</p></section>
    <section class="settings-card"><div class="card-title">${icon('key')}<h2>Модель и ответы</h2></div><label class="field">Провайдер<select id="llm_provider"><option value="lm_studio">LM Studio · локально</option><option value="ollama">Ollama · локально</option><option value="open_router">OpenRouter · облако</option><option value="open_ai">OpenAI API · облако</option><option value="compatible">Другой совместимый API</option></select></label><div class="form-grid">${field('llm_url','Базовый адрес API','Например: https://openrouter.ai/api/v1')}${field('llm_model','Идентификатор модели')}</div>
    <div class="api-key-row"><label class="field">API-ключ<input id="api-key" type="password" autocomplete="off" spellcheck="false" placeholder="Ключ не отображается после сохранения"/></label><button type="button" class="secondary" id="save-api-key">Сохранить ключ</button><button type="button" class="text-button" id="delete-api-key">Удалить ключ</button></div><p id="api-key-status" class="form-note"></p><button type="button" id="get-api-key" class="text-button">Получить API-ключ у провайдера ↗</button>
    <div class="check-grid"><label><input id="llm_fallback" type="checkbox"/> Обращаться к модели, если команда не распознана</label><label><input id="llm_structured" type="checkbox"/> Структурированные ответы JSON · модель должна их поддерживать</label></div><button id="check-llm" type="button" class="secondary">Проверить соединение и модели ${icon('arrow',16)}</button><p id="llm-models" class="form-note"></p><p class="form-note">В облако отправляется текст запроса; распознавание аудио остаётся локальным. API-ключ защищён Windows и хранится отдельно от settings.json. Подписка ChatGPT не заменяет API-ключ: API имеет отдельный учёт, у некоторых тарифов доступны API-кредиты. Ответы раскрываются из панели Васи; их можно закрыть кнопкой × или Escape.</p></section>
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
  for(const key of ['vosk_library','vosk_model','whisper_executable','whisper_model','yandex_app_path','spotify_app_path','piper_executable','piper_model'] as const) next[key]=next[key].replace(/^"(.*)"$/,'$1');
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
    connected=true;
    speechReady = initial.speech_ready; settings = initial.settings; muted = initial.muted; busy = initial.busy; captureOpen = initial.capture_open;
    status = initial.status; dataDirectory = initial.data_directory;
    keySet=initial.llm_key_set;result=initial.last_result||result;
    error = [...initial.errors, ...(initial.last_error ? [initial.last_error] : [])].join('\n');
    render();
  }
  const catalog=await call<Builtin[]>('builtin_commands');if(catalog)builtins=catalog;
  const installed=await call<{id:string;name:string}[]>('list_voices');if(installed)voices=installed;
  if(page==='commands')renderBuiltins();
  await refreshDevices();
  if (!settings.onboarding_complete && page === 'home') render();
  setInterval(async () => {
    try {
      const state = await invoke<Snapshot>('get_state');
      muted = state.muted; busy = state.busy; captureOpen = state.capture_open; status = state.status;
      updateStatus();
    } catch { /* A transient IPC failure is reported on the next user action. */ }
  }, 1000);
}
if (new URLSearchParams(location.search).has('overlay')) { void startIsland(); }
else { shell(); void connect(); }
