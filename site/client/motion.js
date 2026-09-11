import { gsap } from 'gsap';
import { ScrollTrigger } from 'gsap/ScrollTrigger';
import Lenis from 'lenis';
import 'lenis/dist/lenis.css';
import { initDemo } from './terminal.js';

gsap.registerPlugin(ScrollTrigger);
let destroyPage;

export function initMotion() {
  destroyPage?.();
  const cleanups = [];
  const media = gsap.matchMedia();
  const controller = new AbortController();
  const { signal } = controller;
  const listen = (target, event, handler, options = {}) => target?.addEventListener(event, handler, { ...options, signal });
  cleanups.push(initDemo());

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

  // Enhancement starts only after the complete static page is present.
  media.add('(prefers-reduced-motion: no-preference)', () => {
    const restores = [];
    document.querySelectorAll('[data-reveal]').forEach(heading => {
      if (heading.querySelector('a, button, code, strong, em')) return;
      const original = heading.innerHTML;
      const accessible = document.createElement('span');
      accessible.className = 'sr-only';
      accessible.textContent = heading.innerText.replace(/\n/g, ' ');
      const visual = document.createElement('span');
      visual.className = 'reveal-visual';
      visual.setAttribute('aria-hidden', 'true');
      // Preserve intentional line breaks and the heading's color spans.
      visual.innerHTML = original;
      const walker = document.createTreeWalker(visual, NodeFilter.SHOW_TEXT);
      const texts = [];
      while (walker.nextNode()) texts.push(walker.currentNode);
      texts.forEach(node => {
        const fragment = document.createDocumentFragment();
        node.textContent.split(/(\s+)/).forEach(part => {
          if (!part.trim()) fragment.append(document.createTextNode(part));
          else { const span = document.createElement('span'); span.className = 'split-word'; span.textContent = part; fragment.append(span); }
        });
        node.replaceWith(fragment);
      });
      heading.replaceChildren(accessible, visual);
      restores.push(() => { heading.innerHTML = original; });
    });
    const intro = gsap.timeline({ defaults: { ease: 'power3.out' } });
    intro.from('.hero h1 .split-word', { y: 18, duration: .7, stagger: .065 }, 0)
      .from('.hero-copy', { y: 12, duration: .65 }, .12)
      .from('.hero .demo', { y: 20, duration: .8 }, .2);
    document.querySelectorAll('.workflow [data-reveal], .start [data-reveal]').forEach(heading => {
      gsap.from(heading.querySelectorAll('.split-word'), { y: 22, autoAlpha: 0, duration: .6, stagger: .055, ease: 'power3.out', scrollTrigger: { trigger: heading, start: 'top 90%', once: true } });
    });
    return () => restores.forEach(restore => restore());
  });

  media.add('(min-width: 1024px) and (prefers-reduced-motion: no-preference)', () => {
    const layout = document.querySelector('.story-layout');
    const stage = document.querySelector('.story-stage');
    const panels = gsap.utils.toArray('.story-panel');
    const dots = [...document.querySelectorAll('.stage-progress i')];
    const count = document.querySelector('[data-story-count]');
    document.documentElement.classList.add('motion-story');
    gsap.set(panels, { autoAlpha: 0 });
    gsap.set(panels[0], { autoAlpha: 1 });
    let current = 0;
    const show = index => {
      if (index === current) return;
      gsap.to(panels[current], { autoAlpha: 0, y: -9, duration: .2, overwrite: true });
      gsap.fromTo(panels[index], { y: 13 }, { autoAlpha: 1, y: 0, duration: .35, ease: 'power3.out', overwrite: true });
      dots.forEach((dot, i) => dot.classList.toggle('on', i === index));
      count.textContent = `0${index + 1} / 03`;
      current = index;
    };
    ScrollTrigger.create({ trigger: stage, start: 'top 22%', endTrigger: layout, end: 'bottom 80%', pin: true, pinSpacing: false, invalidateOnRefresh: true });
    document.querySelectorAll('[data-story]').forEach((story, index) => {
      ScrollTrigger.create({ trigger: story, start: 'top 52%', end: 'bottom 52%', onEnter: () => show(index), onEnterBack: () => show(index) });
    });
    return () => { document.documentElement.classList.remove('motion-story'); dots.forEach((dot,i) => dot.classList.toggle('on', i === 0)); };
  });

  media.add('(pointer: fine) and (prefers-reduced-motion: no-preference)', () => {
    const lenis = new Lenis({ autoRaf: false, smoothWheel: true, syncTouch: false, duration: .85, anchors: true, prevent: node => Boolean(node.closest('dialog, .side, .code, .install, .table')) });
    const update = () => ScrollTrigger.update();
    const tick = seconds => lenis.raf(seconds * 1000);
    lenis.on('scroll', update);
    gsap.ticker.add(tick);
    gsap.ticker.lagSmoothing(0);
    const visibility = () => document.hidden ? lenis.stop() : lenis.start();
    document.addEventListener('visibilitychange', visibility);
    return () => { document.removeEventListener('visibilitychange', visibility); gsap.ticker.remove(tick); lenis.off('scroll', update); lenis.destroy(); };
  });
  let active = true;
  document.fonts.ready.then(() => { if (active) ScrollTrigger.refresh(); });
  const refresh = () => ScrollTrigger.refresh();
  document.querySelectorAll('img').forEach(img => { if (!img.complete) listen(img, 'load', refresh, { once: true }); });
  destroyPage = () => {
    active = false;
    media.revert();
    controller.abort();
    cleanups.forEach(cleanup => cleanup?.());
    closeMenu();
    destroyPage = undefined;
  };
  // Expose only lifecycle hooks for browser regression checks, no data mutation API.
  window.shepherdMotion = { destroy: () => destroyPage?.(), init: initMotion, inspect: () => ({ triggers: ScrollTrigger.getAll().length, smooth: document.documentElement.classList.contains('lenis') }) };
}
initMotion();
window.addEventListener('pagehide', () => destroyPage?.());
window.addEventListener('pageshow', event => { if (event.persisted) initMotion(); });
