import MarkdownIt from 'markdown-it';
import {invoke, isTauri} from '@tauri-apps/api/core';
import './markdown.css';

const markdown = new MarkdownIt({html: false, breaks: true});
markdown.renderer.rules.image = (tokens, index) => markdown.utils.escapeHtml(tokens[index].content);
markdown.validateLink = href => {
  try {
    const url = new URL(href);
    return (url.protocol === 'https:' || url.protocol === 'http:') && !url.username && !url.password;
  } catch {
    return false;
  }
};

export function renderMarkdown(target: HTMLElement, text: string) {
  target.innerHTML = markdown.render(text);
  target.scrollTop = 0;
  for (const link of target.querySelectorAll('a')) {
    link.target = '_blank';
    link.rel = 'noopener noreferrer';
    link.onclick = event => {
      if (!isTauri()) return;
      event.preventDefault();
      void invoke('open_answer_link', {url: link.href}).catch(error => {
        link.title = String(error);
      });
    };
    // The webview delegates link navigation to the system browser.
    link.onauxclick = event => {
      if (!isTauri()) return;
      event.preventDefault();
      if (event.button === 1) link.click();
    };
  }
}
