export function normalizeEmbedUrl(value) {
  const url = new URL(value.trim());
  if (url.protocol !== 'https:') throw new Error('Use an HTTPS URL for embedded content.');
  if (url.username || url.password) throw new Error('Embed URLs must not contain usernames or passwords.');
  if (url.hostname === 'drive.google.com') {
    const file = url.pathname.match(/^\/file\/d\/([A-Za-z0-9_-]+)\/(?:view|edit|preview)\/?$/);
    if (file) url.pathname = `/file/d/${file[1]}/preview`;
  }
  return { url: url.href, origin: url.origin };
}
