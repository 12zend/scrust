/**
 * Messages from the compiler shown over the stage (in the renderer's overlay
 * container): progress while compiling, errors and unsupported-block notices.
 */

const STYLE = `
.scrust-status {
    position: absolute; left: 0; right: 0; bottom: 0; top: 0;
    display: flex; align-items: center; justify-content: center;
    pointer-events: none; font: 13px/1.4 "Helvetica Neue", Helvetica, Arial, sans-serif;
}
.scrust-status[hidden] { display: none; }
.scrust-status-box {
    pointer-events: auto; max-width: 90%; max-height: 80%; overflow: auto; box-sizing: border-box;
    padding: 10px 12px; border-radius: 6px; background: rgba(30, 30, 36, 0.88); color: #f2f2f5;
    box-shadow: 0 2px 10px rgba(0, 0, 0, 0.3);
}
.scrust-status-box pre { margin: 6px 0 0; white-space: pre-wrap; font: 11px/1.35 ui-monospace, Menlo, monospace; }
.scrust-status-box button {
    float: right; margin-left: 8px; border: 0; background: transparent; color: inherit;
    font-size: 16px; line-height: 1; cursor: pointer;
}
.scrust-status.notice { align-items: flex-end; }
.scrust-status.error .scrust-status-box { background: rgba(120, 24, 24, 0.92); }
.scrust-spinner {
    display: inline-block; width: 12px; height: 12px; margin-right: 8px; vertical-align: -2px;
    border: 2px solid currentColor; border-right-color: transparent; border-radius: 50%;
    animation: scrust-spin 0.8s linear infinite;
}
@keyframes scrust-spin { to { transform: rotate(360deg); } }
body.scrust-live [class*="stage_monitor-wrapper"] { visibility: hidden; }
`;

let styleInstalled = false;

class StatusOverlay {
    /** @param {function(): ?HTMLElement} getContainer where to mount (the renderer's overlay container) */
    constructor (getContainer) {
        this.getContainer = getContainer;
        this.el = null;
        this.timer = null;
    }

    _mount () {
        if (!styleInstalled) {
            const style = document.createElement('style');
            style.textContent = STYLE;
            document.head.appendChild(style);
            styleInstalled = true;
        }
        if (!this.el) {
            this.el = document.createElement('div');
            this.el.hidden = true;
        }
        const container = this.getContainer();
        if (container && this.el.parentNode !== container) {
            container.style.position = container.style.position || 'relative';
            container.appendChild(this.el);
        }
        return this.el;
    }

    _show (kind, title, detail, closable) {
        clearTimeout(this.timer);
        const el = this._mount();
        el.className = `scrust-status ${kind}`;
        el.textContent = '';
        const box = document.createElement('div');
        box.className = 'scrust-status-box';
        if (closable) {
            const close = document.createElement('button');
            close.textContent = '×';
            close.title = 'Close';
            close.onclick = () => this.clear();
            box.appendChild(close);
        }
        if (kind === 'busy') {
            const spinner = document.createElement('span');
            spinner.className = 'scrust-spinner';
            box.appendChild(spinner);
        }
        box.appendChild(document.createTextNode(title));
        if (detail) {
            const pre = document.createElement('pre');
            pre.textContent = detail;
            box.appendChild(pre);
        }
        el.appendChild(box);
        el.hidden = false;
    }

    busy (text) {
        this._show('busy', text, '', false);
    }

    error (detail) {
        this._show('error', 'scrust: コンパイルに失敗しました', detail, true);
    }

    notice (detail) {
        this._show('notice', 'scrust: 未対応のブロックは何もしない命令として実行されます', detail, true);
        this.timer = setTimeout(() => this.clear(), 8000);
    }

    flash (text) {
        this._show('notice', text, '', true);
        this.timer = setTimeout(() => this.clear(), 3000);
    }

    clear () {
        clearTimeout(this.timer);
        if (this.el) this.el.hidden = true;
    }
}

module.exports = StatusOverlay;
