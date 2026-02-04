import { ChangeSet, EditorState } from "@codemirror/state";
import {
  EditorView,
  keymap,
  lineNumbers,
  highlightActiveLine,
  highlightActiveLineGutter,
  drawSelection,
  dropCursor,
  rectangularSelection,
  crosshairCursor,
  highlightSpecialChars,
} from "@codemirror/view";
import {
  history,
  historyKeymap,
  defaultKeymap,
  indentWithTab,
  cursorCharLeft,
  cursorCharRight,
  cursorLineUp,
  cursorLineDown,
  indentMore,
  indentLess,
} from "@codemirror/commands";
import {
  bracketMatching,
  indentOnInput,
  syntaxHighlighting,
  defaultHighlightStyle,
  HighlightStyle,
  syntaxTree,
} from "@codemirror/language";
import { javascript } from "@codemirror/lang-javascript";
import {
  autocompletion,
  completionKeymap,
  startCompletion,
  acceptCompletion,
  completionStatus,
  closeBrackets,
  closeBracketsKeymap,
} from "@codemirror/autocomplete";
import { tags } from "@lezer/highlight";


const theme = EditorView.theme(
  {
    "&": {
      color: "#ffffff",
      backgroundColor: "#1e1e1e",
      fontFamily:
        "JetBrains Mono, SF Mono, Menlo, Monaco, Consolas, monospace",
      fontSize: "13px",
      lineHeight: "1.55",
    },
    ".cm-content": {
      caretColor: "#00ED64",
    },
    ".cm-cursor, .cm-dropCursor": {
      borderLeftColor: "#00ED64",
    },
    ".cm-gutters": {
      backgroundColor: "#252526",
      color: "#858585",
      border: "none",
    },
    ".cm-activeLine": {
      backgroundColor: "#2a2d2e",
    },
    ".cm-activeLineGutter": {
      backgroundColor: "#2a2d2e",
    },
    ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": {
      backgroundColor: "#37373d",
    },
    ".cm-matchingBracket": {
      color: "#00ED64",
      fontWeight: "bold",
    },
    ".cm-tooltip": {
      backgroundColor: "#252526",
      color: "#ffffff",
      border: "1px solid #454545",
      borderRadius: "6px",
      boxShadow: "0 6px 16px rgba(0,0,0,0.35)",
    },
    ".cm-tooltip-autocomplete ul": {
      fontFamily:
        "JetBrains Mono, SF Mono, Menlo, Monaco, Consolas, monospace",
      fontSize: "12px",
      maxHeight: "300px",
    },
    ".cm-tooltip-autocomplete li": {
      padding: "6px 8px",
    },
    ".cm-tooltip-autocomplete li[aria-selected]": {
      backgroundColor: "#2d2d2d",
      color: "#ffffff",
    },
  },
  { dark: true }
);

const highlightStyle = HighlightStyle.define([
  { tag: tags.keyword, color: "#C586C0" },
  { tag: [tags.operator, tags.logicOperator], color: "#D4D4D4" },
  { tag: [tags.name, tags.variableName], color: "#D4D4D4" },
  { tag: [tags.propertyName], color: "#C8E1FF" },
  { tag: [tags.function(tags.variableName)], color: "#56D4DD" },
  { tag: [tags.string], color: "#79B8FF" },
  { tag: [tags.number], color: "#FB8532" },
  { tag: [tags.comment], color: "#959DA5", fontStyle: "italic" },
  { tag: [tags.definition(tags.variableName)], color: "#C8E1FF" },
]);

let view = null;
let pendingCompletion = null;
let pendingTextTimer = null;
let pendingTextDirty = false;
let activeTabId = null;
const tabBuffers = new Map();
let completionIdleTimer = null;
let completionContext = null;
const COMPLETION_IDLE_MS = 200;
const TEXT_CHANGE_DEBOUNCE_MS = 120;

function postIpc(message) {
  if (window.ipc && window.ipc.postMessage) {
    window.ipc.postMessage(JSON.stringify(message));
  }
}

function insertTextAtSelection(view, text) {
  if (!text) return;
  const sel = view.state.selection.main;
  view.dispatch({
    changes: { from: sel.from, to: sel.to, insert: text },
    selection: { anchor: sel.from + text.length },
  });
}

function requestClipboardWrite(text) {
  if (!text) return;
  postIpc({ type: "clipboard_copy", text });
  if (window.isSecureContext && navigator?.clipboard?.writeText) {
    navigator.clipboard.writeText(text).catch(() => {});
  }
}

function requestClipboardRead(view) {
  postIpc({ type: "clipboard_paste" });
  if (window.isSecureContext && navigator?.clipboard?.readText) {
    navigator.clipboard
      .readText()
      .then((text) => {
        if (text) insertTextAtSelection(view, text);
      })
      .catch(() => {});
  }
}

function executeQuery() {
  if (!view) return false;
  flushTextChange();
  postIpc({
    type: "execute_query",
    text: view.state.doc.toString(),
  });
  return true;
}

function handleTab(view) {
  if (completionStatus(view.state) === "active") {
    return acceptCompletion(view);
  }
  return indentMore(view);
}

function mapKind(kind) {
  if (!kind) return "variable";
  switch (kind.toLowerCase()) {
    case "collection":
      return "property";
    case "method":
      return "function";
    case "operator":
      return "keyword";
    default:
      return "variable";
  }
}

function requestCompletions(context, from, to) {
  if (!window.ipc || !window.ipc.postMessage) {
    return Promise.resolve(null);
  }

  const pos = context.pos;
  const doc = context.state.doc;
  const line = doc.lineAt(pos);
  const text = line.text.slice(0, pos - line.from);

  return new Promise((resolve) => {
    completionContext = { context, from, to, resolve, text, line, pos };
    scheduleCompletion();
  });
}

function isInComment(state, pos) {
  const tree = syntaxTree(state);
  let node = tree.resolveInner(pos, -1);
  while (node) {
    const name = node.type && node.type.name;
    if (name === "LineComment" || name === "BlockComment") {
      return true;
    }
    node = node.parent;
  }
  return false;
}

function completionSource(context) {
  if (isInComment(context.state, context.pos)) {
    return null;
  }

  const word = context.matchBefore(/[\w$]+/);
  const from = word ? word.from : context.pos;
  const to = context.pos;

  if (!context.explicit) {
    const prevChar =
      context.pos > 0 ? context.state.sliceDoc(context.pos - 1, context.pos) : "";
    if (prevChar !== "." && prevChar !== "$") {
      const beforeWord = word && word.from > 0
        ? context.state.sliceDoc(word.from - 1, word.from)
        : "";
      if (beforeWord !== "." && beforeWord !== "$") {
        return null;
      }
    }
  }

  return requestCompletions(context, from, to);
}

function scheduleCompletion() {
  if (completionIdleTimer) {
    clearTimeout(completionIdleTimer);
  }
  completionIdleTimer = setTimeout(() => {
    completionIdleTimer = null;
    if (!completionContext || !window.ipc || !window.ipc.postMessage) {
      return;
    }
    const { resolve, from, to, text, line, pos } = completionContext;
    pendingCompletion = { resolve, from, to };
    completionContext = null;
    postIpc({
      type: "completion_request",
      text,
      line: line.number,
      column: pos - line.from + 1,
    });
  }, COMPLETION_IDLE_MS);
}

function scheduleCompletionOnChange(state, update) {
  if (!completionContext) return;
  const pos = state.selection.main.head;
  const line = state.doc.lineAt(pos);
  const text = line.text.slice(0, pos - line.from);
  completionContext = {
    ...completionContext,
    text,
    line,
    pos,
  };
  scheduleCompletion();
}

const updateListener = EditorView.updateListener.of((update) => {
  if (update.docChanged) {
    scheduleTextChange();
    scheduleCompletionOnChange(update.state, update);
  }
});

const controlChars = /[\x00-\x08\x0B\x0C\x0E-\x1F]/g;

const blockControlInput = EditorView.inputHandler.of((view, from, to, text) => {
  if (!text) return false;
  const clean = text.replace(controlChars, "");
  if (clean === text) return false;
  view.dispatch({ changes: { from, to, insert: clean } });
  return true;
});

const filterControlTransactions = EditorState.transactionFilter.of((tr) => {
  if (!tr.docChanged) return tr;
  const changes = [];
  let changed = false;
  tr.changes.iterChanges((fromA, toA, _fromB, _toB, inserted) => {
    const text = inserted.toString();
    const clean = text.replace(controlChars, "");
    if (clean !== text) changed = true;
    changes.push({ from: fromA, to: toA, insert: clean });
  });
  if (!changed) return tr;
  const changeSet = ChangeSet.of(changes, tr.startState.doc.length);
  return tr.startState.update({
    changes,
    selection: tr.newSelection.map(changeSet),
    effects: tr.effects,
    annotations: tr.annotations,
  });
});

const keyHandler = EditorView.domEventHandlers({
  keydown(event, view) {
    const mod = event.metaKey || event.ctrlKey;
    if (mod) {
      const key = event.key.toLowerCase();
      if (key === "c") {
        const sel = view.state.selection.main;
        if (!sel.empty) {
          const text = view.state.sliceDoc(sel.from, sel.to);
          requestClipboardWrite(text);
        }
        event.preventDefault();
        event.stopPropagation();
        return true;
      }
      if (key === "x") {
        const sel = view.state.selection.main;
        if (!sel.empty) {
          const text = view.state.sliceDoc(sel.from, sel.to);
          requestClipboardWrite(text);
          view.dispatch({ changes: { from: sel.from, to: sel.to, insert: "" } });
        }
        event.preventDefault();
        event.stopPropagation();
        return true;
      }
      if (key === "v") {
        requestClipboardRead(view);
        event.preventDefault();
        event.stopPropagation();
        return true;
      }
    }
    if (event.key.startsWith("Arrow")) {
      event.preventDefault();
      event.stopPropagation();
      switch (event.key) {
        case "ArrowLeft":
          return cursorCharLeft(view);
        case "ArrowRight":
          return cursorCharRight(view);
        case "ArrowUp":
          return cursorLineUp(view);
        case "ArrowDown":
          return cursorLineDown(view);
      }
    }
    if (event.key === "Tab") {
      event.preventDefault();
      event.stopPropagation();
      return handleTab(view);
    }
    return false;
  },
});

const baseExtensions = [
  lineNumbers(),
  highlightActiveLineGutter(),
  highlightSpecialChars(),
  history(),
  drawSelection(),
  dropCursor(),
  EditorState.allowMultipleSelections.of(true),
  indentOnInput(),
  filterControlTransactions,
  syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
  syntaxHighlighting(highlightStyle),
  bracketMatching(),
  closeBrackets(),
  autocompletion({
    override: [completionSource],
    activateOnTyping: true,
  }),
  keymap.of([
    { key: "Mod-Enter", run: executeQuery },
    { key: "Ctrl-Enter", run: executeQuery },
    { key: "Ctrl-Space", run: startCompletion },
    { key: "ArrowLeft", run: cursorCharLeft },
    { key: "ArrowRight", run: cursorCharRight },
    { key: "ArrowUp", run: cursorLineUp },
    { key: "ArrowDown", run: cursorLineDown },
    { key: "Tab", run: handleTab },
    { key: "Shift-Tab", run: indentLess },
    indentWithTab,
    ...closeBracketsKeymap,
    ...defaultKeymap,
    ...historyKeymap,
    ...completionKeymap,
  ]),
  EditorView.lineWrapping,
  rectangularSelection(),
  crosshairCursor(),
  blockControlInput,
  keyHandler,
  theme,
  updateListener,
];

const initialDoc = "// MongoDB Shell\n" + "db.";

const state = EditorState.create({
  doc: initialDoc,
  extensions: [
    javascript({ jsx: false, typescript: false }),
    ...baseExtensions,
  ],
});

view = new EditorView({
  state,
  parent: document.getElementById("editor"),
});

function notifyEditorFocus() {
  postIpc({ type: "editor_focus" });
}

view.dom.addEventListener("focusin", notifyEditorFocus);
view.dom.addEventListener("mousedown", notifyEditorFocus);
view.dom.addEventListener("focusout", flushTextChange);

function scheduleTextChange(text) {
  pendingTextDirty = true;
  if (pendingTextTimer) {
    clearTimeout(pendingTextTimer);
  }
  pendingTextTimer = setTimeout(flushTextChange, TEXT_CHANGE_DEBOUNCE_MS);
}

function flushTextChange() {
  if (!pendingTextDirty) return;
  if (pendingTextTimer) {
    clearTimeout(pendingTextTimer);
    pendingTextTimer = null;
  }
  pendingTextDirty = false;
  if (!view || !activeTabId) return;
  tabBuffers.set(activeTabId, view.state.doc.toString());
}

window.receiveSuggestions = function (suggestions) {
  if (!pendingCompletion) return;
  const pending = pendingCompletion;
  pendingCompletion = null;

  if (!suggestions || suggestions.length === 0) {
    pending.resolve(null);
    return;
  }

  const options = suggestions.map((s) => {
    const insertText = s.insert_text || s.label;
    const offset =
      typeof s.cursor_offset === "number" ? s.cursor_offset : null;
    const apply =
      offset !== null
        ? (view, _completion, from, to) => {
            view.dispatch({
              changes: { from, to, insert: insertText },
              selection: { anchor: from + offset },
            });
          }
        : insertText;

    return {
      label: s.label,
      type: mapKind(s.kind),
      apply,
      info: s.documentation || undefined,
    };
  });

  pending.resolve({
    from: pending.from,
    to: pending.to,
    options,
  });
};

window.receiveResult = function (result) {
  console.log("[ForgeEditor] Query result:", result);
};

window.receivePaste = function (text) {
  if (!view || !text) return;
  insertTextAtSelection(view, text);
};

window.setContent = function (content) {
  if (!view) return;
  view.dispatch({
    changes: { from: 0, to: view.state.doc.length, insert: content || "" },
  });
};

window.setActiveTab = function (tabId, content) {
  if (!view || !tabId) return;
  if (activeTabId) {
    tabBuffers.set(activeTabId, view.state.doc.toString());
  }
  activeTabId = tabId;
  const nextContent =
    tabBuffers.get(tabId) ??
    (typeof content === "string" ? content : "");
  view.dispatch({
    changes: { from: 0, to: view.state.doc.length, insert: nextContent },
  });
};


window.flushContentToHost = function (tabId) {
  const targetId = tabId || activeTabId;
  if (!view || !targetId) return;
  tabBuffers.set(targetId, view.state.doc.toString());
  postIpc({
    type: "text_change",
    tab_id: targetId,
    text: view.state.doc.toString(),
  });
};

window.getContent = function () {
  return view ? view.state.doc.toString() : "";
};

window.focusEditor = function () {
  if (view) {
    view.focus();
  }
};

postIpc({ type: "editor_ready" });

setTimeout(() => {
  if (!view) return;
  const end = view.state.doc.length;
  view.dispatch({ selection: { anchor: end } });
  view.focus();
}, 50);
