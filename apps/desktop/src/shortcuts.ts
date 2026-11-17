/** Raccourcis clavier globaux de la régie. */

/** Éléments qui ont leur propre action au clavier (Entrée, Espace) ou qui reçoivent du texte. */
const INTERACTIVE_SELECTOR = [
  "input",
  "textarea",
  "select",
  "button",
  "a[href]",
  "summary",
  "[contenteditable]:not([contenteditable='false'])",
  ...["button", "option", "radio", "checkbox", "switch", "menuitem", "tab", "link", "slider", "textbox", "combobox"].map(
    (r) => `[role='${r}']`,
  ),
].join(",");

/**
 * Vrai si la touche doit revenir à l'élément ciblé plutôt qu'aux raccourcis globaux :
 * un champ de saisie, ou un bouton / lien qui a le focus (Entrée et Espace l'activent ;
 * les détourner déclencherait une coupure à l'antenne à la place de son action).
 */
export function isInteractiveTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  if (target instanceof HTMLElement && target.isContentEditable) return true;
  return target.closest(INTERACTIVE_SELECTOR) !== null;
}
