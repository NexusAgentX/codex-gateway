import type { InputHTMLAttributes } from "react";
import { fieldName } from "../../lib/format";
import { Input } from "./form";

type NumberInputProps = Omit<InputHTMLAttributes<HTMLInputElement>, "onChange" | "value" | "type"> & {
  label: string;
  value: string;
  onChange: (value: string) => void;
};

export function NumberInput({ label, value, onChange, name, min = "0", ...props }: NumberInputProps) {
  return <Input name={name ?? fieldName(label)} value={value} onChange={(event) => onChange(event.target.value)} type="number" min={min} aria-label={label} placeholder={label} {...props} />;
}
