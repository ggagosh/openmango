import type { ReactNode } from 'react';
import { colors, layout } from '../../theme/index.ts';

interface ModalProps {
  title: string;
  width?: number;
  height?: number;
  children?: ReactNode;
}

export function Modal({ title, width = layout.modal.defaultWidth, height, children }: ModalProps) {
  return (
    <box position="absolute" width="100%" height="100%" justifyContent="center" alignItems="center">
      <box
        width={width}
        height={height}
        border
        borderStyle={layout.borderStyle}
        borderColor={colors.borderFocused}
        backgroundColor={colors.background}
        flexDirection="column"
      >
        <box paddingLeft={1} paddingRight={1}>
          <text fg={colors.foreground}>
            <b>{title}</b>
          </text>
        </box>
        <box flexDirection="column" padding={1} flexGrow={1}>
          {children}
        </box>
      </box>
    </box>
  );
}
