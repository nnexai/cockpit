const WIDGET_CSP = "default-src 'none'; script-src http: https: data: blob: 'unsafe-inline'; style-src http: https: data: blob: 'unsafe-inline'; img-src http: https: data: blob:; font-src http: https: data: blob:; media-src http: https: data: blob:; connect-src http: https: ws: wss: data: blob:; frame-src 'none'; child-src 'none'; worker-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";
const MAX_SELECTION_BYTES = 16 * 1024;

function scriptLiteral(value: unknown): string {
  const serialized = JSON.stringify(value);
  if (serialized === undefined) throw new TypeError("Widget bridge state must be JSON");
  return serialized
    .replaceAll("<", "\\u003c")
    .replaceAll(">", "\\u003e")
    .replaceAll("&", "\\u0026")
    .replaceAll("\u2028", "\\u2028")
    .replaceAll("\u2029", "\\u2029");
}

/** Embeds preflighted, trusted author HTML without the Library's inert sanitizer.
 * CSP permits ordinary external resources while withholding ambient host schemes.
 * The caller must keep the frame opaque with sandbox="allow-scripts" only.
 */
export function widgetDocument(html: string, bridge: {
  nonce: string;
  revision: number;
  selection: unknown | null;
  hasSelection?: boolean;
}): string {
  const document = new DOMParser().parseFromString(html, "text/html");
  const charset = document.createElement("meta");
  charset.setAttribute("charset", "utf-8");
  const policy = document.createElement("meta");
  policy.setAttribute("http-equiv", "Content-Security-Policy");
  policy.setAttribute("content", WIDGET_CSP);
  const script = document.createElement("script");
  script.textContent = `(() => {
    'use strict';
    const state = JSON.parse(${scriptLiteral(JSON.stringify({ ...bridge, hasSelection: bridge.hasSelection ?? bridge.selection !== null }))});
    const post = parent.postMessage.bind(parent);
    const stringify = JSON.stringify.bind(JSON);
    const encoder = new TextEncoder();
    const encode = encoder.encode.bind(encoder);
    const freeze = Object.freeze;
    const finite = Number.isFinite;
    const values = Object.values;
    const clock = Date.now.bind(Date);
    const isArray = Array.isArray;
    const pending = [state.selection];
    while (pending.length) {
      const value = pending.pop();
      if (value && typeof value === 'object' && !Object.isFrozen(value)) {
        pending.push(...values(value));
        freeze(value);
      }
    }
    const select = freeze(function select(value) {
      const value_json = stringify(value, (_key, item) => {
        if (typeof item === 'undefined' || typeof item === 'function' || typeof item === 'symbol'
          || typeof item === 'bigint' || (typeof item === 'number' && !finite(item))) {
          throw new TypeError('Widget selections must contain only finite JSON values');
        }
        return item;
      });
      if (typeof value_json !== 'string') throw new TypeError('Widget selection must be JSON');
      if (encode(value_json).byteLength > ${MAX_SELECTION_BYTES}) {
        throw new RangeError('Widget selection exceeds 16 KiB');
      }
      post({type: 'cockpit.widget.select', nonce: state.nonce, revision: state.revision, value_json}, '*');
    });
    Object.defineProperty(window, 'cockpit', {
      value: freeze({select, selection: state.selection, hasSelection: state.hasSelection}),
      writable: false, configurable: false, enumerable: true
    });
    // srcdoc inherits Cockpit's base URL, so bare fragments otherwise navigate
    // to the host page. Keep native fragment scrolling, focus and history.
    window.addEventListener('click', event => {
      if (event.defaultPrevented || event.button !== 0 || event.ctrlKey
        || event.altKey || event.shiftKey || event.metaKey) return;
      const element = event.target instanceof Element ? event.target : event.target?.parentElement;
      const anchor = element?.closest('a[href], area[href]');
      if (!anchor || anchor.hasAttribute('download')) return;
      const target = (anchor.getAttribute('target') || '').toLowerCase();
      if (target && target !== '_self') return;
      const href = anchor.getAttribute('href').trim();
      if (href.startsWith('#')) anchor.setAttribute('href', 'about:srcdoc' + href);
    }, true);
    let prefixUntil = 0;
    window.addEventListener('message', event => {
      if (event.source !== parent) return;
      const message = event.data;
      if (!message || typeof message !== 'object' || isArray(message)
        || message.type !== 'cockpit.widget.prefix' || message.nonce !== state.nonce
        || message.revision !== state.revision || typeof message.active !== 'boolean') return;
      prefixUntil = message.active ? clock() + 2000 : 0;
    });
    const namedKeys = Object.freeze({Dead: true, Escape: true, Tab: true, Enter: true,
      ArrowUp: true, ArrowDown: true, ArrowLeft: true, ArrowRight: true});
    const namedCodes = Object.freeze({Escape: true, Tab: true, Enter: true,
      ArrowUp: true, ArrowDown: true, ArrowLeft: true, ArrowRight: true,
      Space: true, Minus: true, Equal: true,
      BracketLeft: true, BracketRight: true, Backslash: true, Semicolon: true, Quote: true,
      Backquote: true, Comma: true, Period: true, Slash: true});
    const hasOwn = Object.prototype.hasOwnProperty;
    const intent = event => {
      if (event.isTrusted) {
        post({type: 'cockpit.widget.intent', nonce: state.nonce, revision: state.revision}, '*');
      }
    };
    window.addEventListener('pointerdown', intent, true);
    window.addEventListener('keydown', event => {
      intent(event);
      if (!event.isTrusted || event.isComposing || event.metaKey) return;
      const keyAllowed = /^[^\\u0000-\\u001f\\u007f-\\u009f]$/u.test(event.key) || hasOwn.call(namedKeys, event.key);
      const codeAllowed = /^(Key[A-Z]|Digit[0-9])$/.test(event.code) || hasOwn.call(namedCodes, event.code);
      if (!keyAllowed || !codeAllowed) return;
      const prefix = event.ctrlKey && !event.altKey && !event.shiftKey
        && event.key.toLowerCase() === 'b';
      const armed = prefixUntil > clock();
      if (prefix && armed) {
        prefixUntil = 0;
      } else {
        if (!event.ctrlKey && !event.altKey && !armed) return;
        prefixUntil = prefix && !event.repeat ? clock() + 2000 : 0;
        event.preventDefault();
        event.stopImmediatePropagation();
      }
      post({type: 'cockpit.widget.shortcut', nonce: state.nonce, revision: state.revision,
        key: event.key, code: event.code, ctrlKey: event.ctrlKey, altKey: event.altKey,
        shiftKey: event.shiftKey, metaKey: event.metaKey, repeat: event.repeat}, '*');
    }, true);
  })();`;
  const style = document.createElement("style");
  style.textContent = 'html { color-scheme: dark; } body { margin: 0; padding: 16px 32px 40px; background: #0C1016; color: #D8DEE8; font: 14px/22px "IBM Plex Sans", "Noto Sans", sans-serif; overflow-wrap: anywhere; } pre { overflow: auto; } table { border-collapse: collapse; } th, td { padding: 4px 8px; border: 1px solid #2A3340; }';
  document.head.prepend(charset, policy, script, style);
  return `<!doctype html>${document.documentElement.outerHTML}`;
}
