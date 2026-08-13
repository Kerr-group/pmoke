function extractDocumentUrls(document) {
  const urls = [];
  const attributePattern = /\b(?:href|content)=["']([^"']+)["']/gi;
  const sitemapPattern = /<loc>([^<]+)<\/loc>/gi;
  for (const match of document.matchAll(attributePattern)) urls.push(match[1].trim());
  for (const match of document.matchAll(sitemapPattern)) urls.push(match[1].trim());
  return urls;
}

export function hasExactUrl(document, expected) {
  const expectedUrl = new URL(expected);
  return extractDocumentUrls(document).some((candidate) => {
    try {
      return new URL(candidate).href === expectedUrl.href;
    } catch {
      return false;
    }
  });
}
