import { forwardRef, type InputHTMLAttributes, type SelectHTMLAttributes, type TextareaHTMLAttributes } from "react";
import { cn } from "../../lib/utils";

const fieldClass =
  "min-h-9 w-full min-w-0 rounded-md border border-zinc-300 bg-white px-3 py-2 text-sm text-zinc-900 outline-none transition focus:border-emerald-700 focus:ring-2 focus:ring-emerald-700/10 disabled:cursor-not-allowed disabled:opacity-55";

export const Input = forwardRef<HTMLInputElement, InputHTMLAttributes<HTMLInputElement>>(({ className, ...props }, ref) => (
  <input ref={ref} className={cn(fieldClass, className)} {...props} />
));
Input.displayName = "Input";

export const Select = forwardRef<HTMLSelectElement, SelectHTMLAttributes<HTMLSelectElement>>(({ className, ...props }, ref) => (
  <select ref={ref} className={cn(fieldClass, className)} {...props} />
));
Select.displayName = "Select";

export const Textarea = forwardRef<HTMLTextAreaElement, TextareaHTMLAttributes<HTMLTextAreaElement>>(({ className, ...props }, ref) => (
  <textarea ref={ref} className={cn(fieldClass, "min-h-24 resize-y", className)} {...props} />
));
Textarea.displayName = "Textarea";

export function Field({ label, children, className }: { label: string; children: React.ReactNode; className?: string }) {
  return (
    <label className={cn("grid gap-1.5 text-xs font-semibold text-zinc-600", className)}>
      {label}
      {children}
    </label>
  );
}

export function CheckboxLine({ children, className }: { children: React.ReactNode; className?: string }) {
  return <label className={cn("inline-flex min-h-9 items-center gap-2 text-sm font-semibold text-zinc-600", className)}>{children}</label>;
}
