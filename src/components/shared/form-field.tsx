import type { ReactNode } from 'react';
import { colors } from '../../theme/index.ts';

interface FormFieldProps {
  label: string;
  required?: boolean;
  error?: string;
  children: ReactNode;
}

export function FormField({ label, required, error, children }: FormFieldProps) {
  return (
    <box flexDirection="column" marginBottom={1}>
      <box marginBottom={0}>
        <text fg={colors.foreground}>
          {label}
          {required && <span fg={colors.danger}>*</span>}
        </text>
      </box>
      {children}
      {error && <text fg={colors.statusError}>{error}</text>}
    </box>
  );
}
