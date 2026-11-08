/** Add responsive panel controls and keyboard tab semantics to the existing tab owner. */
export function wirePanelNavigation(tabs: NodeListOf<HTMLButtonElement>, resize: () => void, closeEditor: () => void): void {
  const panel = document.getElementById('panel')!;
  const toggle = document.getElementById('panelToggle')!;
  const setOpen = (open: boolean) => {
    if (!open) closeEditor();
    document.body.classList.toggle('panel-collapsed', !open);
    panel.hidden = !open;
    toggle.setAttribute('aria-expanded', String(open));
    toggle.textContent = open ? 'Hide controls' : 'Show controls';
    if (!open && panel.contains?.(document.activeElement)) toggle.focus();
    resize();
  };
  toggle.addEventListener('click', () => setOpen(panel.hidden));
  if (window.matchMedia?.('(max-width: 700px)').matches) setOpen(false);
  const sync = (selected: HTMLButtonElement) => tabs.forEach(tab => {
    const active = tab === selected;
    tab.tabIndex = active ? 0 : -1;
    tab.setAttribute('aria-selected', String(active));
    const content = document.getElementById(tab.dataset['tab']!);
    if (content) content.hidden = !active;
  });
  tabs.forEach(tab => {
    tab.id = `nav-${tab.dataset['tab']}`;
    tab.setAttribute('role', 'tab'); tab.setAttribute('aria-controls', tab.dataset['tab']!);
    const content = document.getElementById(tab.dataset['tab']!);
    content?.setAttribute('role', 'tabpanel'); content?.setAttribute('aria-labelledby', tab.id);
    if (content) content.tabIndex = 0;
    tab.addEventListener('click', () => sync(tab));
    tab.addEventListener('keydown', event => {
      const visible = Array.from(tabs).filter(button => !button.hidden && !button.disabled);
      const index = visible.indexOf(tab);
      const next = event.key === 'Home' ? visible[0] : event.key === 'End' ? visible.at(-1)
        : event.key === 'ArrowRight' ? visible[(index + 1) % visible.length]
        : event.key === 'ArrowLeft' ? visible[(index - 1 + visible.length) % visible.length] : undefined;
      if (next) { event.preventDefault(); event.stopPropagation(); next.click(); next.focus(); }
    });
  });
  if (tabs[0]) sync(tabs[0]);
  if (typeof MutationObserver !== 'undefined') {
    const capabilityChanges = new MutationObserver(() => {
      const selected = Array.from(tabs).find(tab => tab.getAttribute('aria-selected') === 'true');
      if (!selected?.hidden) return;
      const moveFocus = selected === document.activeElement || document.getElementById(selected.dataset['tab']!)?.contains(document.activeElement);
      const next = Array.from(tabs).find(tab => !tab.hidden);
      next?.click(); if (moveFocus) next?.focus();
    });
    tabs.forEach(tab => capabilityChanges.observe(tab, {attributes:true,attributeFilter:['hidden']}));
  }
  document.querySelectorAll<HTMLButtonElement>('[data-open-tab]').forEach(button => {
    button.addEventListener('click', () => {
      const tab = Array.from(tabs).find(tab => tab.dataset['tab'] === button.dataset['openTab']);
      tab?.click(); tab?.focus();
    });
  });
}

/** Keep Tab within an active modal without intercepting editing/gameplay keys. */
export function containModalTab(event: KeyboardEvent, root: HTMLElement): void {
  if (event.key !== 'Tab') return;
  const controls = Array.from(root.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex="0"], summary'))
    .filter(element => !element.hidden && element.getClientRects().length > 0);
  const first = controls[0]; const last = controls.at(-1);
  if (!first || !last) return;
  if (event.shiftKey && (document.activeElement === first || !root.contains(document.activeElement))) { event.preventDefault(); last.focus(); }
  else if (!event.shiftKey && (document.activeElement === last || !root.contains(document.activeElement))) { event.preventDefault(); first.focus(); }
}
