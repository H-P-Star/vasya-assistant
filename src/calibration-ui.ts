import {invoke} from '@tauri-apps/api/core';
import type {Settings} from './types';
type Report={wake_detected?:boolean;expected:string;vosk_text:string;refined_text:string;rms:number;peak:number;clipped_percent:number;suggested_gain:number;matches:boolean;wake_candidate:string|null;message:string};
const esc=(s:string)=>s.replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]!));
export function calibrationUI(root:HTMLElement,initial:Settings,native:boolean,onSave:(next:Settings)=>Promise<void>){
 const settings=structuredClone(initial);let step=0;let working=false;const reports:Report[]=[];
 const words=settings.wake_words.slice(0,2);if(words.length===1)words.push(words[0]);
 const steps=[{title:'Фоновый шум',phrase:''},...Array.from({length:4},(_,i)=>({title:`Вызов ${i+1} из 4`,phrase:words[i%2]})),{title:'Команда по-русски',phrase:'включи трек Перемен исполнителя Кино'},{title:'Английское название',phrase:'включи трек Numb исполнителя Linkin Park'}];
 root.innerHTML=`<section class="settings-card"><h2>Настроим распознавание под ваш микрофон</h2><p>Мастер измерит шум и громкость, проверит обращения и сохранит выбранные исправления. Нейросеть не переобучается. Голосовые команды во время записи не выполняются; записи остаются только в памяти.</p><label class="field">Микрофон<select id="cal-mic"><option value="">По умолчанию Windows</option></select></label><label class="field">Распознавание при проверке<select id="cal-mode"><option value="whisper">Vosk + Whisper · русский и английский</option><option value="vosk">Только Vosk · русский</option></select></label><div id="cal-step"></div><div class="command-toolbar"><button class="primary" id="cal-record">Записать шаг · 6 секунд</button><button class="secondary" id="cal-next">Пропустить шаг</button><button class="secondary" id="cal-cancel" hidden>Остановить запись</button></div><p id="cal-status" role="status"></p><div id="cal-results" class="cal-results"></div></section>
 <section class="settings-card"><h2>Персональный профиль</h2><div id="cal-suggestions"></div><div class="check-grid"><label><input type="checkbox" id="cal-gain" checked/> Применить рекомендованное усиление и уровень шума</label><label><input type="checkbox" id="cal-whisper" checked/> Использовать Whisper с автоматическим языком для команд</label></div><label class="field">Исправления распознавания<textarea id="cal-dictionary" rows="5" placeholder="линкин парк => Linkin Park"></textarea><small>Одна замена в строке: как слышит Вася => нужный текст. Можно добавить названия треков, приложений и английские слова.</small></label><button class="primary" id="cal-save">Сохранить профиль</button><p id="cal-save-status" role="status"></p></section>`;
 const el=(id:string)=>root.querySelector<HTMLElement>('#'+id)!;
 (el('cal-mode') as HTMLSelectElement).value=settings.speech_mode;
 (el('cal-dictionary') as HTMLTextAreaElement).value=settings.speech_corrections.map(c=>`${c.heard} => ${c.replacement}`).join('\n');
 if(native)void invoke<string[]>('list_microphones').then(names=>{const select=el('cal-mic') as HTMLSelectElement;select.innerHTML='<option value="">По умолчанию Windows</option>'+[...new Set([...names,...(settings.microphone?[settings.microphone]:[])])].map(n=>`<option value="${esc(n)}">${esc(n)}</option>`).join('');select.value=settings.microphone;}).catch(e=>el('cal-status').textContent=String(e));
 function renderStep(){const current=steps[Math.min(step,steps.length-1)];el('cal-step').innerHTML=step>=steps.length?'<p>Шаги завершены. Проверьте профиль ниже и сохраните его.</p>':`<p class="eyebrow">ШАГ ${step+1} / ${steps.length} · ${current.title}</p><label class="field">${step===0?'Во время записи сохраняйте тишину':'Произнесите обычным голосом'}<input id="cal-expected" value="${esc(current.phrase)}" ${step===0?'readonly':''}/></label>`;(el('cal-record') as HTMLButtonElement).disabled=step>=steps.length;(el('cal-next') as HTMLButtonElement).disabled=step>=steps.length;}
 function renderSuggestions(){const counts=new Map<string,number>();for(const r of reports)if(r.wake_candidate&&!settings.wake_words.includes(r.wake_candidate))counts.set(r.wake_candidate,(counts.get(r.wake_candidate)||0)+1);el('cal-suggestions').innerHTML=[...counts].filter(([,n])=>n>=2).map(([word,n])=>`<label class="cal-alias"><input type="checkbox" data-wake="${esc(word)}"/> Добавить «${esc(word)}» как вариант вызова · ${n} совпадения</label>`).join('')+'<p class="form-note">Варианты вызова добавляются только после вашего выбора. Частые обычные слова могут давать случайные активации.</p>';}
 el('cal-next').onclick=()=>{if(!working){step++;renderStep();}};
 el('cal-cancel').onclick=()=>{if(native)void invoke('cancel_current');};
 el('cal-record').onclick=async()=>{
  if(!native){el('cal-status').textContent='Калибровка доступна в Windows-приложении.';return;}
  const expected=step===0?'':(el('cal-expected') as HTMLInputElement).value.trim();if(step>0&&!expected){el('cal-status').textContent='Введите фразу для проверки.';return;}
  settings.microphone=(el('cal-mic') as HTMLSelectElement).value;settings.speech_mode=(el('cal-mode') as HTMLSelectElement).value as Settings['speech_mode'];settings.speech_language='auto';
  working=true;(el('cal-record') as HTMLButtonElement).disabled=true;(el('cal-next') as HTMLButtonElement).disabled=true;el('cal-cancel').hidden=false;el('cal-status').textContent='Записываю… '+(expected?'Говорите после нажатия кнопки.':'Сохраняйте тишину.');
  const timer=setTimeout(()=>el('cal-status').textContent='Распознаю запись локально…',step===0?3000:6000);
  try{const report=await invoke<Report>('calibrate_speech',{settings,expected,seconds:step===0?3:6});reports.push(report);el('cal-status').textContent=report.message;
   const row=document.createElement('div');row.className='cal-report';row.innerHTML=`<strong>${esc(report.expected||'Фоновый шум')}</strong><p>${report.wake_detected?'Детектор вызова: сработал<br/>':''}Vosk: ${esc(report.vosk_text||'—')}<br/>Whisper: ${esc(report.refined_text||'—')}</p><small>Пик ${(report.peak*100).toFixed(1)}% · шум/сигнал ${(report.rms*100).toFixed(2)}% · перегрузка ${report.clipped_percent.toFixed(1)}%</small>`;el('cal-results').append(row);step++;renderStep();renderSuggestions();
   if(report.expected&&report.expected.split(' ').length>2&&!report.matches&&report.vosk_text){const textarea=el('cal-dictionary') as HTMLTextAreaElement;const line=`${report.refined_text||report.vosk_text} => ${report.expected}`;textarea.value+=(textarea.value?'\n':'')+line;}
  }catch(e){el('cal-status').textContent=String(e);}finally{clearTimeout(timer);working=false;el('cal-cancel').hidden=true;(el('cal-record') as HTMLButtonElement).disabled=step>=steps.length;(el('cal-next') as HTMLButtonElement).disabled=step>=steps.length;}
 };
 el('cal-save').onclick=async()=>{
  if(!native){el('cal-save-status').textContent='Профиль сохраняется в Windows-приложении.';return;}
  if(working){el('cal-save-status').textContent='Дождитесь завершения записи.';return;}
  try{const next=structuredClone(settings);next.wake_aliases=[...new Set([...settings.wake_aliases,...[...root.querySelectorAll<HTMLInputElement>('[data-wake]:checked')].map(i=>i.dataset.wake!)])];
   next.speech_corrections=(el('cal-dictionary') as HTMLTextAreaElement).value.split('\n').filter(l=>l.trim()).map(line=>{const at=line.indexOf('=>');if(at<1||!line.slice(at+2).trim())throw Error('Исправление: используйте формат «как услышано => нужный текст».');return{heard:line.slice(0,at).trim(),replacement:line.slice(at+2).trim()};});
   if((el('cal-gain') as HTMLInputElement).checked){const gains=reports.filter(r=>r.expected&&r.peak>0.001&&r.clipped_percent<1).map(r=>r.suggested_gain).sort((a,b)=>a-b);if(gains.length)next.speech_gain=gains[Math.floor(gains.length/2)];const noise=reports.find(r=>!r.expected);if(noise)next.noise_floor=Math.min(0.2,Math.max(0.0005,noise.rms*next.speech_gain*1.8));}
   if((el('cal-whisper') as HTMLInputElement).checked){next.speech_mode='whisper';next.speech_language='auto';}
   await onSave(next);el('cal-save-status').textContent='Профиль сохранён. Распознавание перезапускается.';
  }catch(e){el('cal-save-status').textContent=String(e);}
 };
 renderStep();renderSuggestions();
 return ()=>{if(working&&native)void invoke('cancel_current').catch(()=>{});};
}
