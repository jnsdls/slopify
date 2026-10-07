// Runs in every frame and does its work in the SDK's iframe only. The SDK plays through a <video>
// there, so WebKit's Now Playing entry and the media keys belong to that frame's media session.
// Actions go to Rust; metadata comes down from player.html through postMessage.
(() => {
  if (location.origin !== 'https://sdk.scdn.co') return;
  const post = (action) =>
    window.webkit.messageHandlers.ipc.postMessage(JSON.stringify({ t: 'media-action', action }));
  for (const action of ['play', 'pause', 'stop', 'nexttrack', 'previoustrack']) {
    navigator.mediaSession.setActionHandler(action, () => post(action));
  }
  window.addEventListener('message', (e) => {
    if (e.source !== window.parent || !e.data || !('slopifyNowPlaying' in e.data)) return;
    const metadata = e.data.slopifyNowPlaying;
    navigator.mediaSession.metadata = metadata && new MediaMetadata(metadata);
  });
})();
