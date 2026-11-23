import DOMPurify from 'dompurify';
import katex from 'katex';
import { Marked } from 'marked';
import { isSafeLink } from './reveal.js';
const marked = new Marked({
    gfm: true,
    breaks: true,
});
marked.use({
    extensions: [
        {
            name: 'displayMath',
            level: 'block',
            start(source) {
                const dollar = source.indexOf('$$');
                const bracket = source.indexOf('\\[');
                const candidates = [dollar, bracket].filter((index) => index >= 0);
                return candidates.length ? Math.min(...candidates) : undefined;
            },
            tokenizer(source) {
                const match = source.match(/^\$\$([\s\S]+?)\$\$(?:\n|$)/) ??
                    source.match(/^\\\[([\s\S]+?)\\\](?:\n|$)/);
                if (!match)
                    return undefined;
                return {
                    type: 'displayMath',
                    raw: match[0],
                    expression: match[1].trim(),
                };
            },
            renderer(token) {
                return renderFormula(String(token.expression), true);
            },
        },
        {
            name: 'inlineMath',
            level: 'inline',
            start(source) {
                const dollar = source.indexOf('$');
                const paren = source.indexOf('\\(');
                const candidates = [dollar, paren].filter((index) => index >= 0);
                return candidates.length ? Math.min(...candidates) : undefined;
            },
            tokenizer(source) {
                const match = source.match(/^\$([^$\n]+?)\$/) ??
                    source.match(/^\\\((.+?)\\\)/);
                if (!match)
                    return undefined;
                return {
                    type: 'inlineMath',
                    raw: match[0],
                    expression: match[1].trim(),
                };
            },
            renderer(token) {
                return renderFormula(String(token.expression), false);
            },
        },
    ],
});
export function renderSafeMarkdown(source) {
    const rendered = marked.parse(source);
    const html = typeof rendered === 'string' ? rendered : '';
    return DOMPurify.sanitize(html, {
        USE_PROFILES: { html: true, mathMl: true, svg: false },
        FORBID_TAGS: ['style', 'script', 'iframe', 'object', 'embed', 'form'],
        FORBID_ATTR: ['style', 'srcdoc'],
    });
}
/** Route link activation to the host without navigating the conversation view. */
export function handleMarkdownLink(event, capabilities, onError) {
    const link = event.composedPath().find((node) => (node instanceof HTMLAnchorElement || node instanceof HTMLAreaElement) &&
        node.hasAttribute('href'));
    if (!link)
        return false;
    if (event.defaultPrevented || event.button > 1)
        return true;
    event.preventDefault();
    if (!isSafeLink(link.href))
        return true;
    const request = { url: link.href, source: 'markdown' };
    if (capabilities.openLink) {
        try {
            void Promise.resolve(capabilities.openLink(request)).catch(onError);
        }
        catch (error) {
            onError(error);
        }
    }
    else {
        link.dispatchEvent(new CustomEvent('agent-conversation-link-open', {
            bubbles: true,
            composed: true,
            detail: request,
        }));
    }
    return true;
}
function renderFormula(expression, displayMode) {
    try {
        return katex.renderToString(expression, {
            displayMode,
            output: 'mathml',
            throwOnError: true,
            trust: false,
            strict: 'error',
        });
    }
    catch {
        return `<code class="formula-error">${escapeHtml(expression)}</code>`;
    }
}
function escapeHtml(value) {
    return value
        .replaceAll('&', '&amp;')
        .replaceAll('<', '&lt;')
        .replaceAll('>', '&gt;')
        .replaceAll('"', '&quot;')
        .replaceAll("'", '&#039;');
}
//# sourceMappingURL=markdown.js.map