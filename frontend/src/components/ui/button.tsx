import { forwardRef, type ButtonHTMLAttributes } from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "../../lib/utils";

const buttonVariants = cva(
  "inline-flex min-h-9 items-center justify-center gap-2 rounded-md border px-3 py-2 text-sm font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-55",
  {
    variants: {
      variant: {
        default: "border-zinc-300 bg-white text-zinc-800 hover:bg-zinc-50",
        primary: "border-emerald-800 bg-emerald-800 text-white hover:bg-emerald-900",
        ghost: "border-transparent bg-transparent text-zinc-700 hover:bg-zinc-100",
        link: "min-h-0 border-transparent bg-transparent p-0 font-semibold text-emerald-800 hover:text-emerald-950"
      },
      size: {
        default: "",
        icon: "h-9 w-9 p-0",
        sm: "min-h-8 px-2 py-1.5"
      }
    },
    defaultVariants: {
      variant: "default",
      size: "default"
    }
  }
);

export type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & VariantProps<typeof buttonVariants>;

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(({ className, variant, size, ...props }, ref) => {
  return <button ref={ref} className={cn(buttonVariants({ variant, size }), className)} {...props} />;
});
Button.displayName = "Button";
