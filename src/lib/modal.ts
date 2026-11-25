/** Open a native modal, keep keyboard focus inside it, and restore the opener on close. */
export function modal(dialog: HTMLDialogElement, initialFocusSelector?: string) {
  const opener = document.activeElement;
  dialog.showModal();
  if (initialFocusSelector) dialog.querySelector<HTMLElement>(initialFocusSelector)?.focus();

  function cycleFocus(event: KeyboardEvent) {
    if (event.key !== 'Tab') return;
    const controls = [...dialog.querySelectorAll<HTMLElement>(
      'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href], summary, [tabindex="0"]'
    )].filter(element => element.getClientRects().length > 0);
    const first = controls[0], last = controls.at(-1);
    if (!first || (event.shiftKey ? document.activeElement === first : document.activeElement === last)) {
      event.preventDefault();
      (event.shiftKey ? last : first)?.focus();
    }
  }

  dialog.addEventListener('keydown', cycleFocus);
  return {
    destroy() {
      dialog.removeEventListener('keydown', cycleFocus);
      dialog.close();
      if (opener instanceof HTMLElement && opener.isConnected) opener.focus({preventScroll: true});
    }
  };
}
