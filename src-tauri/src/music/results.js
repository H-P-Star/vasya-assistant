(() => {
  const blocked = document.querySelector('[class*="BlockPage_title"]')?.innerText.trim();
  if (blocked) return {blocked};
  const rows = [];
  const seen = new Set();
  const selector = [
    '[class*="CommonTrack_root"]', '[class*="Track_root"]', '[class*="TrackCard_root"]',
    '[class*="ArtistCard_root"]', '[class*="PlaylistCard_root"]', '[class*="EntityCard_root"]',
    '[class*="HorizontalCardContainer_root"]', '.d-track'
  ].join(',');
  const playingControl = '[class*="WithCover_root_playing"],[class*="WithPosition_root_playing"],button[aria-label^="Пауза"],button[aria-label^="Приостанов"],button[aria-label^="Pause" i]';
  let selectedPlaying = '';
  for (const root of document.querySelectorAll(selector)) {
    if (root.matches('[class*="_root_disabled"]')) continue;
    const primary = root.querySelector('a[class*="_titleLink"],.d-track__title a');
    const trackTitle = root.querySelector('[class*="Meta_title__"],[class*="TrackCard_title__"],[data-test-id="TRACK_TITLE"],.d-track__title');
    const entityRoute = root.matches('[class*="ArtistCard_root"]') ? '/artist/' :
      root.matches('[class*="PlaylistCard_root"]') ? '/playlists/' : '/track/';
    const link = primary || [...root.querySelectorAll('a[href]')].find(a => a.getAttribute('href')?.includes(entityRoute) && a.textContent.trim());
    const path = link?.getAttribute('href') || '';
    // Each row has one identity. Artist links under the track title are credits.
    const kind = path.includes('/artist/') ? 'artist' : path.includes('/playlists/') ? 'playlist'
      : trackTitle || path.includes('/track/') ? 'track' : '';
    const title = (kind === 'track' ? trackTitle || link : link)?.textContent.trim();
    if (!kind || !title) continue;
    const credits = [...root.querySelectorAll('a[href*="/artist/"],[class*="Meta_artistCaption"],.d-track__artists')];
    const artist = kind === 'track' ? [...new Set(credits.map(el => el.textContent.trim()).filter(Boolean))].join(', ') : '';
    const id = JSON.stringify([kind, path, title, artist]);
    if (seen.has(id)) continue;
    seen.add(id);
    root.dataset.vasyaCandidate = id;
    rows.push({id, kind, title, artist});
    if (root.querySelector(playingControl)) selectedPlaying = id;
  }
  const header = document.querySelector('[data-vasya-header]');
  if (header?.querySelector(playingControl)) selectedPlaying = header.dataset.vasyaHeader;
  const meta = navigator.mediaSession?.metadata;
  const player = document.querySelector('[class*="PlayerBarDesktop_root"],.player-controls');
  const playing = Boolean(selectedPlaying || player?.querySelector(playingControl)
    || navigator.mediaSession?.playbackState === 'playing'
    || [...document.querySelectorAll('audio,video')].some(m => !m.paused && !m.ended && m.readyState >= 2));
  return {rows, playback:{playing, selectedPlaying,
    title:meta?.title || player?.querySelector('[class*="Meta_title__"],a[href*="/track/"]')?.textContent.trim() || '',
    artist:meta?.artist || player?.querySelector('a[href*="/artist/"]')?.textContent.trim() || ''}};
})()
