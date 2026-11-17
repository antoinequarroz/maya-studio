/** Champ texte qui garde un brouillon pendant l'édition et valide sur Entrée ou à la sortie du champ. */

import { type InputHTMLAttributes, useRef, useState } from "react";

/**
 * Entrée valide, Échap annule ; dans les deux cas le champ perd le focus et c'est
 * `onBlur` qui fait l'unique validation. Le brouillon est tenu dans une référence :
 * `blur()` déclenche `onBlur` de façon synchrone, avant que React n'ait rendu
 * le `setDraft` précédent, et la fermeture verrait sinon un brouillon périmé.
 */
export function DraftInput({
  value,
  onCommit,
  type = "text",
  ...rest
}: {
  value: string;
  onCommit: (v: string) => void;
  type?: "text" | "number";
} & Omit<InputHTMLAttributes<HTMLInputElement>, "value" | "onChange" | "type" | "onBlur" | "onKeyDown">) {
  const [draft, setDraftState] = useState<string | null>(null);
  const draftRef = useRef<string | null>(null);
  const setDraft = (v: string | null) => {
    draftRef.current = v;
    setDraftState(v);
  };
  const commit = () => {
    const d = draftRef.current;
    setDraft(null);
    if (d !== null && d !== value) onCommit(d);
  };
  return (
    <input
      {...rest}
      type={type}
      value={draft ?? value}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          (e.target as HTMLInputElement).blur();
        } else if (e.key === "Escape") {
          setDraft(null);
          (e.target as HTMLInputElement).blur();
        }
      }}
    />
  );
}
