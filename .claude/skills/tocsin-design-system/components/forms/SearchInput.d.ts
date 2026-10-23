/**
 * Text input: library search, API key fields. Gold border + glow on focus.
 * @startingPoint section="Forms" subtitle="Search / key input with gold focus" viewport="700x200"
 */
export interface SearchInputProps {
  value: string;
  onChange?: (v: string) => void;
  placeholder?: string;
  /** Default 240px (library toolbar). */
  width?: string;
}
export function SearchInput(props: SearchInputProps): JSX.Element;
