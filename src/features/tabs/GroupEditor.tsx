import { useId, useState, type FormEvent } from 'react';
import { api, errorMessage, type GroupEditorRequest } from '../../lib/ipc';
import { announce } from '../../lib/ui';
import { ColorPicker, IconPicker } from '../bookmarks/StylePickers';
import controls from '../../components/controls.module.css';
import form from '../../components/popupForm.module.css';

/** Name, icon and colour of a tab group, like a bookmark folder's. Rendered in the overlay. */
export function GroupEditor({
  request,
  onClose,
  onPickerOpen,
}: {
  request: GroupEditorRequest;
  onClose: () => void;
  onPickerOpen: () => void;
}) {
  const id = useId();
  const [title, setTitle] = useState(request.title);
  const [icon, setIcon] = useState<string | null>(request.icon);
  const [color, setColor] = useState(request.color);
  const [error, setError] = useState('');

  const save = async (e: FormEvent) => {
    e.preventDefault();
    try {
      // An empty icon removes it.
      await api.groups.update(request.groupId, { title: title.trim(), color, icon: icon ?? '' });
      announce('Group saved');
      onClose();
    } catch (err) {
      setError(errorMessage(err));
    }
  };

  return (
    <form className={form.editor} onSubmit={(e) => void save(e)} aria-labelledby={`${id}-h`}>
      <h2 id={`${id}-h`} className={form.heading}>
        Edit group
      </h2>
      <label className={form.field}>
        <span>Name</span>
        <input
          className={controls.input}
          value={title}
          autoFocus
          maxLength={60}
          placeholder="Unnamed group"
          onChange={(e) => setTitle(e.target.value)}
        />
      </label>
      <div className={form.field}>
        <span>Icon</span>
        <IconPicker value={icon} color={color} allowNone label="Group icon" onChange={setIcon} />
      </div>
      <div className={form.field}>
        <span>Colour</span>
        <ColorPicker value={color} allowNone={false} label="Group colour" onChange={(c) => c && setColor(c)} onPickerOpen={onPickerOpen} />
      </div>
      {error && (
        <p className={form.error} role="alert">
          {error}
        </p>
      )}
      <div className={form.actions}>
        <button type="button" className={controls.button} onClick={onClose}>
          Cancel
        </button>
        <button type="submit" className={`${controls.button} ${controls.primary}`}>
          Save
        </button>
      </div>
    </form>
  );
}
