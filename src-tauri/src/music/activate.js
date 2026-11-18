// Called in the renderer with data arguments. Locate a point for a real mouse gesture.
function(id, kind, title, stage) {
  let root = [...document.querySelectorAll('[data-vasya-candidate]')].find(el => el.dataset.vasyaCandidate === id);
  if (!root && kind !== 'track') {
    const header = document.querySelector('[class*="PageHeaderArtist_root"],[class*="CommonPageHeader_root"],main header');
    const name = header?.querySelector('h1,h2')?.textContent.trim();
    if (name && name.toLocaleLowerCase() === title.toLocaleLowerCase()) {
      header.dataset.vasyaHeader = id;
      root = header;
    }
  }
  if (!root) return null;
  const center = el => {
    const r = el.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0) return null;
    return {x:r.left+r.width/2,y:r.top+r.height/2};
  };
  if (stage === 'hover') {
    root.scrollIntoView({block:'nearest'});
    return center(root.querySelector('[class*="playButtonCell"],[class*="coverBlock"]') || root);
  }
  const controls = [...root.querySelectorAll('button,[role="button"],.d-track__play')];
  const name = b => (b.getAttribute('aria-label') || b.title || '').trim();
  const play = controls.find(b => !b.disabled && b.getAttribute('aria-disabled') !== 'true' &&
    (/^(воспроизве[дс]|играть|слушать|play\b)/i.test(name(b))
     || (!/пауза|pause|приостанов/i.test(name(b)) && b.matches('[class*="_playButton"],[class*="_playControl"],.d-track__play'))));
  const point = play && center(play);
  if (stage !== 'alternate' && point && play.contains(document.elementFromPoint(point.x,point.y))) return {...point, action:'click'};
  if (kind === 'track' && root.matches('[class*="CommonTrack_root"],[class*="EntityCard_root"],[class*="HorizontalCardContainer_root"]')) {
    // SearchTrackCard and Track dispatch playback from the row's click detail 2.
    // Use its own surface, outside links, menus and other controls.
    const r = root.getBoundingClientRect();
    for (const fraction of [.5,.68,.85,.3]) {
      const x = r.left+r.width*fraction, y = r.top+r.height/2;
      const target = document.elementFromPoint(x,y);
      if (target && root.contains(target) && !target.closest('a,button,[role="button"]')) return {x,y,action:'row'};
    }
  }
  if (kind !== 'track' && !root.dataset.vasyaHeader) {
    const link = root.querySelector(kind==='artist' ? 'a[href*="/artist/"]' : 'a[href*="/playlists/"]');
    const p = link && center(link);
    if (p && link.contains(document.elementFromPoint(p.x,p.y))) return {...p,action:'open'};
  }
  return null;
}
