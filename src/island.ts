import {invoke,isTauri} from '@tauri-apps/api/core';
import {listen} from '@tauri-apps/api/event';
import './island.css';
import {renderMarkdown} from './markdown';
type AssistantEvent={kind:string;text:string;level:number};
export async function startIsland(){
 document.documentElement.classList.add('island-page');
 document.body.innerHTML=`<div class="island-shell" role="region" aria-label="Вася">
  <div class="island-pill"><div class="island-symbol"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="9" y="2" width="6" height="12" rx="3"/><path d="M5 10v2a7 7 0 0014 0v-2M12 19v3"/></svg></div>
   <button class="island-caption" id="island-expand" title="Развернуть ответ"><strong id="island-title">Слушаю</strong><span id="island-detail">Произнесите команду</span></button>
   <div class="island-meter" aria-hidden="true">${'<i></i>'.repeat(7)}</div>
   <button class="island-close" id="island-close" aria-label="Закрыть панель и остановить ответ" title="Закрыть · Esc">×</button></div>
  <section class="island-answer" aria-label="Ответ Васи"><div class="island-answer-heading"><span>ВАСЯ · ОТВЕТ</span><button id="island-collapse">Свернуть ↑</button></div>
   <article id="island-answer-text" class="markdown-body" tabindex="0"></article>
   <footer><span id="island-copy-status" role="status">Ответ сохранён до следующего результата</span><button id="island-main">Открыть Васю</button><button id="island-copy">Скопировать</button></footer></section>
 </div>`;
 const shell=document.querySelector<HTMLElement>('.island-shell')!;
 const title=document.getElementById('island-title')!;const detail=document.getElementById('island-detail')!;const output=document.getElementById('island-answer-text')!;
 const expandButton=document.getElementById('island-expand') as HTMLButtonElement;
 let active=!isTauri(),expanded=false,result='';
 const layout=(next:boolean)=>{expanded=next;shell.classList.toggle('expanded',next);};
 async function expand(next:boolean){if(!result)return;layout(next);if(isTauri())await invoke('expand_island',{expanded:next});}
 async function dismiss(){active=false;layout(false);if(isTauri())await invoke('dismiss_island');else shell.hidden=true;}
 document.getElementById('island-close')!.onclick=()=>{void dismiss();};
 document.getElementById('island-collapse')!.onclick=()=>{void expand(false);};
 expandButton.onclick=()=>{void expand(!expanded);};
 window.addEventListener('keydown',e=>{if(e.key==='Escape'){e.preventDefault();void dismiss();}});
 document.getElementById('island-main')!.onclick=()=>{if(isTauri())void invoke('show_main');};
 document.getElementById('island-copy')!.onclick=async()=>{try{if(isTauri())await invoke('copy_result');else await navigator.clipboard.writeText(result);document.getElementById('island-copy-status')!.textContent='Скопировано';}catch{document.getElementById('island-copy-status')!.textContent='Не удалось скопировать. Откройте ответ в Васе.';}};
 if(!isTauri()){detail.textContent='Предпросмотр · нажмите для примера';result='### Что такое IPv6\n\n**IPv6** — версия интернет-протокола с большим пространством адресов.\n\nПример адреса: `2001:db8::1`. Голосовой ответ можно скопировать прямо из панели.';renderMarkdown(output,result);return;}
 await listen<boolean>('island-layout',({payload})=>layout(payload));
 await listen<AssistantEvent>('assistant',({payload:e})=>{
  if(e.kind==='activated'){active=true;result='';layout(false);shell.className='island-shell listening';title.textContent='Слушаю';detail.textContent='Произнесите команду';output.textContent='';expandButton.disabled=true;return;}
  if(e.kind==='deactivated'||(e.kind==='microphone'&&e.text==='muted')){active=false;layout(false);return;}
  if(!active)return;
  if(e.kind==='level'){document.querySelectorAll<HTMLElement>('.island-meter i').forEach((bar,i)=>{bar.style.height=`${4+Math.min(1,e.level)*(14+Math.abs(Math.sin(i*1.9))*13)}px`;});return;}
  if(e.kind==='heard'){title.textContent='Выполняю';detail.textContent=e.text;shell.classList.add('thinking');}
  if(e.kind==='status'){
   if(e.text.startsWith('Ищу в интернете')||e.text.startsWith('Сверяю источники')||e.text.startsWith('Уточняю распознавание')){title.textContent=e.text;shell.classList.add('thinking');}
   else if(e.text.includes('модели')){title.textContent='Думаю';shell.classList.add('thinking');}
   else if(e.text.startsWith('Слушаю')){title.textContent='Слушаю';shell.classList.remove('thinking');}
   else if(e.text.startsWith('Диктовка ·')){title.textContent='Диктую';detail.textContent='«Стоп диктовка» — завершить';}
  }
  if(e.kind==='result'||e.kind==='answer'||e.kind==='preview_result'){result=e.text;renderMarkdown(output,result);expandButton.disabled=false;if(e.kind!=='result'){layout(true);title.textContent='Вася отвечает';detail.textContent='Полный ответ ниже';shell.classList.remove('thinking');}}
  if(e.kind==='info'&&!expanded){title.textContent=result?'Текст готов':'Готово';detail.textContent=result?'Нажмите, чтобы прочитать':e.text;shell.classList.remove('thinking');}
  if(e.kind==='error'){title.textContent='Не получилось';detail.textContent=e.text;shell.classList.remove('thinking');shell.classList.add('failed');}
 });
}
