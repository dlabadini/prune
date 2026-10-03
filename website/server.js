const express = require('express');
const fs = require('fs');
const path = require('path');
const { marked } = require('marked');

const app = express();
const PORT = process.env.PORT || 3000;

const read = (...parts) => fs.readFileSync(path.join(__dirname, ...parts), 'utf8');
const prune = JSON.parse(read('data', 'prune.json'));
const pageTemplate = read('views', 'page.html');
const faq = JSON.parse(read('data', 'faq.json'));

const escapeHtml = (text) =>
  text.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);

// The home page's visible FAQ and its FAQPage JSON-LD come from the same data, so they always match.
const faqHtml = faq
  .map(({ q, a }) => {
    const answer = escapeHtml(a).replace(/`([^`]+)`/g, '<code>$1</code>');
    return `          <details>\n            <summary>${escapeHtml(q)}</summary>\n            <p>${answer}</p>\n          </details>`;
  })
  .join('\n');
const faqLd = JSON.stringify({
  '@type': 'FAQPage',
  '@id': 'https://prunecode.ai/#faq',
  mainEntity: faq.map(({ q, a }) => ({
    '@type': 'Question',
    name: q,
    acceptedAnswer: { '@type': 'Answer', text: a.replace(/`/g, '') },
  })),
});
// The docs embed the agent runbook (content/setup.md) as a section, demoted one heading level.
const setupSection = marked
  .parse(read('content', 'setup.md'))
  .replace(/<h1>[\s\S]*?<\/h1>/, '')
  .replace(/<(\/?)h3>/g, '<$1h4>')
  .replace(/<(\/?)h2>/g, '<$1h3>');
const docsHtml = read('views', 'docs.html').replace('<!--SETUP-->', setupSection);

const homeHtml = read('views', 'index.html').replace('<!--FAQ-->', faqHtml).replace('<!--FAQ_LD-->', faqLd);

// Pages that exist as both HTML and Markdown. Agents asking for `Accept: text/markdown` get the
// Markdown; everyone else gets HTML. `/x.md` always returns the Markdown.
const pages = {
  '/': { html: 'index.html', md: 'index.md' },
  '/docs': { html: 'docs.html', md: 'docs.md' },
  '/setup': { md: 'setup.md', title: 'Agent setup', description: 'Step-by-step instructions a coding agent follows to install Prune, add the pre-commit check, connect MCP, and work the first tasks.' },
  '/about': { md: 'about.md', title: 'About', description: 'Why Prune exists, how it is built, and who maintains it.' },
  '/contact': { md: 'contact.md', title: 'Contact', description: 'Report bugs and false positives, ask questions, or reach the Prune maintainers.' },
  '/privacy': { md: 'privacy.md', title: 'Privacy', description: 'Prune collects no telemetry and your code never leaves your machine. How this website handles analytics.' },
};

function wantsMarkdown(req) {
  return req.accepts(['text/html', 'text/markdown']) === 'text/markdown';
}

function sendMarkdown(res, file, status = 200) {
  res.status(status).type('text/markdown; charset=utf-8').send(read('content', file));
}

function renderPage(route, page) {
  const body = marked.parse(read('content', page.md));
  return pageTemplate
    .replaceAll('{{title}}', page.title)
    .replaceAll('{{description}}', page.description)
    .replaceAll('{{path}}', route)
    .replaceAll('{{robots}}', page.robots || 'index, follow')
    .replace('{{content}}', body);
}

app.disable('x-powered-by');

app.get('/install.sh', (req, res) => {
  res.type('text/plain').sendFile(path.join(__dirname, 'public', 'install.sh'));
});

for (const [route, page] of Object.entries(pages)) {
  app.get(route, (req, res) => {
    res.vary('Accept');
    res.set('Link', '</llms.txt>; rel="describedby"; type="text/plain", </openapi.json>; rel="service-desc"; type="application/json"');
    if (wantsMarkdown(req)) return sendMarkdown(res, page.md);
    if (route === '/') return res.type('html').send(homeHtml);
    if (route === '/docs') return res.type('html').send(docsHtml);
    if (page.html) return res.sendFile(path.join(__dirname, 'views', page.html));
    res.type('html').send(renderPage(route, page));
  });
  const mdRoute = route === '/' ? '/index.md' : `${route}.md`;
  app.get(mdRoute, (req, res) => sendMarkdown(res, page.md));
}

// Public, read-only API: facts agents need to install and configure Prune. Spec: /openapi.json.
const api = express.Router();
const apiError = (res, status, code, message, hint) =>
  res.status(status).json({ error: { code, message, hint, docs: 'https://prunecode.ai/openapi.json' } });

api.use((req, res, next) => {
  res.set('Access-Control-Allow-Origin', '*');
  res.set('Cache-Control', 'public, max-age=300');
  next();
});

api.get('/version', (req, res) => {
  res.json({
    name: prune.name,
    version: prune.version,
    license: prune.license,
    repository: prune.repository,
    releases: `${prune.repository}/releases`,
  });
});

api.get('/install', (req, res) => {
  const { platform } = req.query;
  if (platform === undefined) return res.json({ install: prune.install });
  const known = ['macos', 'linux', 'windows'];
  if (!known.includes(platform)) {
    return apiError(res, 400, 'invalid_platform', `Unknown platform "${platform}".`, `Use one of: ${known.join(', ')}.`);
  }
  res.json({ install: prune.install.filter((method) => method.platforms.includes(platform)) });
});

api.get('/detectors', (req, res) => res.json({ detectors: prune.detectors }));

api.get('/detectors/:id', (req, res) => {
  const detector = prune.detectors.find((item) => item.id === req.params.id);
  if (!detector) {
    return apiError(res, 404, 'detector_not_found', `No detector with id "${req.params.id}".`, `Known ids: ${prune.detectors.map((item) => item.id).join(', ')}.`);
  }
  res.json(detector);
});

api.get('/mcp', (req, res) => res.json(prune.mcp));

api.use((req, res) => {
  apiError(res, 404, 'not_found', `No API endpoint at ${req.method} ${req.originalUrl}.`, 'See /openapi.json for the available endpoints.');
});

api.use((err, req, res, next) => {
  apiError(res, 500, 'internal_error', 'Something went wrong on our side.', 'Retry the request; if it keeps failing, open an issue on GitHub.');
});

app.use('/api/v1', api);
app.use('/api', (req, res) => {
  apiError(res, 404, 'not_found', `No API endpoint at ${req.method} ${req.originalUrl}.`, 'The API lives under /api/v1. See /openapi.json.');
});

app.use(express.static(path.join(__dirname, 'public'), { extensions: ['html'] }));

// 404 in whatever format the client asked for.
app.use((req, res) => {
  res.vary('Accept');
  const type = req.accepts(['text/html', 'text/markdown', 'application/json']);
  if (type === 'text/markdown') return sendMarkdown(res, '404.md', 404);
  if (type === 'application/json') {
    return apiError(res, 404, 'not_found', `No page at ${req.originalUrl}.`, 'See https://prunecode.ai/llms.txt or https://prunecode.ai/sitemap.xml.');
  }
  res.status(404).type('html').send(
    renderPage(req.path, { md: '404.md', title: 'Not found', description: 'This page does not exist.', robots: 'noindex' })
  );
});

// Local dev only
if (require.main === module) {
  app.listen(PORT, () => {
    console.log(`Server running at http://localhost:${PORT}`);
  });
}

module.exports = app;
