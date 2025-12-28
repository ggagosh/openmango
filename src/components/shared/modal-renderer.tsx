import { useModal } from '../../context/modal-context.tsx';
import { AddConnectionModal } from '../connection/add-connection-modal.tsx';
import { CreateDatabaseModal } from '../database/create-database-modal.tsx';
import { CreateCollectionModal } from '../collection/create-collection-modal.tsx';
import { CopyDatabaseModal } from '../database/copy-database-modal.tsx';
import { CopyCollectionModal } from '../collection/copy-collection-modal.tsx';
import { ConfirmDialog } from './confirm-dialog.tsx';

export function ModalRenderer() {
  const { currentModal, closeModal } = useModal();

  if (!currentModal) {
    return null;
  }

  const props = currentModal.props ?? {};

  switch (currentModal.type) {
    case 'add-connection':
      return <AddConnectionModal onClose={closeModal} />;

    case 'create-database':
      return (
        <CreateDatabaseModal
          connectionAlias={props.connectionAlias as string}
          onClose={closeModal}
        />
      );

    case 'create-collection':
      return (
        <CreateCollectionModal
          databaseName={props.databaseName as string}
          connectionAlias={props.connectionAlias as string}
          onClose={closeModal}
        />
      );

    case 'copy-database':
      return (
        <CopyDatabaseModal
          sourceName={props.sourceName as string}
          sourceConnection={props.sourceConnection as string}
          onClose={closeModal}
        />
      );

    case 'copy-collection':
      return (
        <CopyCollectionModal
          sourceName={props.sourceName as string}
          sourceDatabase={props.sourceDatabase as string}
          sourceConnection={props.sourceConnection as string}
          onClose={closeModal}
        />
      );

    case 'confirm':
      return (
        <ConfirmDialog
          title={props.title as string}
          message={props.message as string}
          confirmLabel={props.confirmLabel as string | undefined}
          cancelLabel={props.cancelLabel as string | undefined}
          destructive={props.destructive as boolean | undefined}
          onConfirm={() => {
            (props.onConfirm as (() => void) | undefined)?.();
            closeModal();
          }}
          onCancel={closeModal}
        />
      );

    default:
      return null;
  }
}
