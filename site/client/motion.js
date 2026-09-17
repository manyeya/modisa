// Keep the live product preview and navigation independent of scroll effects.
import { initDemo } from './terminal.js';
let destroyPage;

export function initMotion() {
  destroyPage?.();
  const controller = new AbortController();
  const { signal } = controller;
  const listen = (target, event, handler, options = {}) => target?.addEventListener(event, handler, { ...options, signal });
  const destroyDemo = initDemo();
  const menuToggle = document.querySelector('[data-nav-toggle]');
  const menu = document.querySelector('#mobile-nav');
  const closeMenu = (focus = false) => {
    if (!menu) return;
    menu.hidden = true;
    menuToggle.setAttribute('aria-expanded', 'false');
    menuToggle.setAttribute('aria-label', 'Open navigation');
    if (focus) menuToggle.focus();
  };
  listen(menuToggle, 'click', () => {
    menu.hidden = !menu.hidden;
    menuToggle.setAttribute('aria-expanded', String(!menu.hidden));
    menuToggle.setAttribute('aria-label', menu.hidden ? 'Open navigation' : 'Close navigation');
  });
  listen(menu, 'click', event => { if (event.target.closest('a')) closeMenu(); });
  listen(document, 'keydown', event => { if (event.key === 'Escape' && !menu?.hidden) closeMenu(true); });
  listen(document, 'click', event => { if (!event.target.closest('.landing-top')) closeMenu(); });
  listen(window, 'resize', () => { if (innerWidth > 820) closeMenu(); });

  destroyPage = () => {
    controller.abort();
    destroyDemo?.();
    closeMenu();
    destroyPage = undefined;
  };
  window.modisaMotion = { destroy: () => destroyPage?.(), init: initMotion, inspect: () => ({ triggers: 0, smooth: false }) };
}
initMotion();
window.addEventListener('pagehide', () => destroyPage?.());
window.addEventListener('pageshow', event => { if (event.persisted) initMotion(); });
