import { randomUUID } from 'expo-crypto';
import { useEffect, useState, useSyncExternalStore } from 'react';
import { Linking, Pressable, Text, View } from 'react-native';
import { Button, Field, Notice } from './components';
import { messageOf, type PendingQuestion, type QuestionAnswer, type QuestionMarkdownLink, type QuestionReply, type Result } from './domain';
import { QuestionMarkdown, questionWebUrl } from './QuestionMarkdown';
import { pickQuestionFiles, type PickedQuestionFile } from './questionFiles';
import { color, styles as s } from './theme';

type StagedFile = {
  id: string;
  file: PickedQuestionFile;
  state: 'queued' | 'uploading' | 'ready' | 'failed' | 'removing';
};

export class QuestionDraft {
  readonly answers: QuestionAnswer[];
  readonly files: StagedFile[][];
  readonly fileErrors: string[];
  transferring = false;
  private revision = 0;
  private readonly listeners = new Set<() => void>();

  constructor(count: number) {
    this.answers = Array.from({ length: count }, () => ({ selected: [], text: '' }));
    this.files = Array.from({ length: count }, () => []);
    this.fileErrors = Array.from({ length: count }, () => '');
  }

  snapshot = () => this.revision;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };
  changed() {
    this.revision += 1;
    this.listeners.forEach(listener => listener());
  }
}

export function QuestionCard({ request, draftKey, drafts, canSend, disabled, submit, stageFile, canOpenProject, previewFile }: Readonly<{
  request: PendingQuestion;
  draftKey: string;
  drafts: Map<string, QuestionDraft>;
  canSend: boolean;
  disabled: boolean;
  submit: (reply: QuestionReply) => void;
  stageFile: (position: number, id: string, file?: PickedQuestionFile) => Promise<Result<unknown>>;
  canOpenProject: boolean;
  previewFile: (id: string, label: string) => void;
}>) {
  const [expanded, setExpanded] = useState(true);
  const [linkError, setLinkError] = useState(false);
  const [draft] = useState(() => drafts.get(draftKey) ?? new QuestionDraft(request.questions.length));
  useSyncExternalStore(draft.subscribe, draft.snapshot, draft.snapshot);
  useEffect(() => {
    drafts.delete(draftKey);
    drafts.set(draftKey, draft);
    if (drafts.size > 50) {
      const oldest = drafts.keys().next();
      if (!oldest.done) drafts.delete(oldest.value);
    }
  }, [draft, draftKey, drafts]);
  const answers = draft.answers.map((answer, position) => {
    const ids = draft.files[position]?.filter(entry => entry.state === 'ready').map(entry => entry.id);
    return ids?.length ? { ...answer, attachments: ids } : answer;
  });
  const locked = disabled || !canSend || request.responding;
  const sizeExceeded = answers.some(answer => new TextEncoder().encode(answer.text).byteLength > 8192);
  const complete = !sizeExceeded && !draft.transferring
    && draft.files.every(files => files.every(entry => entry.state === 'ready'))
    && answers.every(answer => answer.selected.length || answer.text.trim() || answer.attachments?.length);

  function update(position: number, answer: QuestionAnswer) {
    const { selected, text } = answer;
    draft.answers[position] = { selected, text };
    draft.changed();
  }

  function openLink(target: QuestionMarkdownLink, label: string) {
    setLinkError(false);
    if (target.kind === 'project') {
      if (canOpenProject) previewFile(target.id, label);
      return;
    }
    const url = questionWebUrl(target.url);
    if (url) void Linking.openURL(url).catch(() => setLinkError(true));
  }

  async function transfer(position: number, entries: StagedFile[], upload: boolean) {
    if (locked || draft.transferring) return;
    draft.transferring = true;
    draft.fileErrors[position] = '';
    try {
      for (const entry of entries) {
        const previous = entry.state;
        entry.state = upload ? 'uploading' : 'removing';
        draft.changed();
        try {
          const result = await stageFile(position, entry.id, upload ? entry.file : undefined);
          if (!result.ok) throw new Error(result.error.message);
          if (upload) entry.state = 'ready';
          else draft.files[position] = draft.files[position]?.filter(file => file !== entry) ?? [];
        } catch (error) {
          entry.state = upload ? 'failed' : previous;
          draft.fileErrors[position] = messageOf(error);
        }
      }
    } finally {
      draft.transferring = false;
      draft.changed();
    }
  }

  async function attach(position: number, kind: 'files' | 'photos') {
    if (locked || draft.transferring) return;
    try {
      const files = await pickQuestionFiles(kind);
      if (drafts.get(draftKey) !== draft || draft.transferring || !files.length) return;
      if (draft.files.reduce((count, items) => count + items.length, 0) + files.length > 100) {
        throw new Error('Add at most 100 files to the answer.');
      }
      const entries = files.map((file): StagedFile => ({ id: randomUUID(), file, state: 'queued' }));
      draft.files[position]?.push(...entries);
      await transfer(position, entries, true);
    } catch (error) {
      draft.fileErrors[position] = messageOf(error);
      draft.changed();
    }
  }

  return (
    <View style={s.permission} testID="pending-question">
      <Pressable
        accessibilityRole="button"
        accessibilityState={{ expanded }}
        aria-expanded={expanded}
        onPress={() => setExpanded(!expanded)}
        style={[s.row, s.between, { minHeight: 44 }]}
      >
        <Text style={[s.subheading, { marginBottom: 0, flexShrink: 1 }]}>Question from the agent</Text>
        <Text accessibilityElementsHidden importantForAccessibility="no" style={s.text}>{expanded ? '⌃' : '⌄'}</Text>
      </Pressable>
      {expanded ? <>
        {request.questions.map((question, position) => {
          const answer = answers[position] ?? { selected: [], text: '' };
          return (
            <View key={position} style={position === 0 ? { gap: 10 } : s.section}>
              {question.header ? <Text style={[s.label, { marginBottom: 0 }]}>{question.header}</Text> : null}
              <QuestionMarkdown text={question.question} nodes={request.markdown?.[position]?.question} open={openLink} canOpenProject={canOpenProject} />
              {question.multiSelect ? <Text style={s.small}>Choose one or more</Text> : null}
              {question.options.map((option, index) => {
                const checked = answer.selected.includes(index);
                return (
                  <View key={index} style={{ gap: 5 }}>
                    <Pressable
                      accessibilityRole={question.multiSelect ? 'checkbox' : 'radio'}
                      accessibilityState={{ checked, disabled: locked }}
                      aria-checked={checked}
                      disabled={locked}
                      onPress={() => update(position, {
                        ...answer,
                        selected: question.multiSelect
                          ? checked ? answer.selected.filter(value => value !== index) : [...answer.selected, index]
                          : [index],
                      })}
                      style={({ pressed }) => [s.button, { alignItems: 'flex-start' }, checked && { borderColor: color.accent }, pressed && s.pressed]}
                    >
                      <Text style={s.buttonText}>{option.label}</Text>
                      {option.description ? <Text style={s.small}>{option.description}</Text> : null}
                    </Pressable>
                    {option.preview ? <View style={{ paddingHorizontal: 12 }}>
                      <QuestionMarkdown text={option.preview} nodes={request.markdown?.[position]?.previews[index]} muted open={openLink} canOpenProject={canOpenProject} />
                    </View> : null}
                  </View>
                );
              })}
              {question.allowCustom ? <>
                <Field label="Custom answer" value={answer.text} maxLength={8192} multiline editable={!locked}
                  testID={`phone-question-answer-${position}`} onChange={text => update(position, { ...answer, text })} />
                {request.can_attach ? <>
                  <View style={s.row}>
                    <Button quiet disabled={locked || draft.transferring} onPress={() => void attach(position, 'files')}>Attach files</Button>
                    <Button quiet disabled={locked || draft.transferring} onPress={() => void attach(position, 'photos')}>Attach photos</Button>
                  </View>
                  {draft.files[position]?.map(entry => (
                    <View key={entry.id} style={[s.button, { alignItems: 'stretch', gap: 6 }]}>
                      <Text style={s.text} selectable>{entry.file.name}</Text>
                      <View style={[s.row, { alignItems: 'center' }]}>
                        {entry.state === 'ready' ? <Text style={s.small}>Ready</Text> : null}
                        {entry.state === 'uploading' ? <Text style={s.small} accessibilityLiveRegion="polite">Uploading…</Text> : null}
                        {entry.state === 'removing' ? <Text style={s.small} accessibilityLiveRegion="polite">Removing…</Text> : null}
                        {entry.state === 'failed' || entry.state === 'queued'
                          ? <Button quiet disabled={locked || draft.transferring} onPress={() => void transfer(position, [entry], true)}>Retry</Button>
                          : null}
                        <Button quiet disabled={locked || draft.transferring} onPress={() => void transfer(position, [entry], false)}>Remove</Button>
                      </View>
                    </View>
                  ))}
                  {draft.fileErrors[position] ? <Notice error>{draft.fileErrors[position]}</Notice> : null}
                </> : null}
              </> : null}
            </View>
          );
        })}
        {sizeExceeded ? <Notice error>Shorten each answer to 8 KiB.</Notice> : null}
        {linkError ? <Notice error>Could not open this link.</Notice> : null}
        {request.error ? <Notice error>{request.error}</Notice> : null}
        {!canSend ? <Notice>Send prompts was not granted for this device.</Notice> : null}
        <View style={s.row}>
          <Button primary disabled={locked || !complete} onPress={() => submit({ type: 'answer', answers })}>
            {request.responding ? 'Sending answers…' : 'Send answers'}
          </Button>
          <Button quiet disabled={locked || draft.transferring} onPress={() => submit({ type: 'skip' })}>Skip</Button>
        </View>
      </> : null}
    </View>
  );
}
