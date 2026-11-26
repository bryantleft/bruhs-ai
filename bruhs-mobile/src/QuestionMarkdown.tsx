import type { ReactNode } from 'react';
import { Platform, Text, View, type TextStyle } from 'react-native';
import type { QuestionMarkdownLink, QuestionMarkdownNode } from './domain';
import { color, styles as s } from './theme';

type OpenLink = (target: QuestionMarkdownLink, label: string) => void;
const inlineStyles: Record<string, TextStyle> = {
  strong: { fontFamily: 'NeonMedium', fontWeight: '600' },
  emphasis: { fontStyle: 'italic' },
  strike: { textDecorationLine: 'line-through' },
  code: { backgroundColor: color.raised, fontFamily: 'Neon' },
};

export function questionWebUrl(raw: string): string | null {
  try {
    const url = new URL(raw);
    return (url.protocol === 'https:' || url.protocol === 'http:') && !url.username && !url.password
      ? url.href : null;
  } catch {
    return null;
  }
}

function label(nodes: QuestionMarkdownNode[]): string {
  return nodes.map(node => typeof node === 'string' ? node : label(node.children)).join('');
}

function inline(node: QuestionMarkdownNode, key: number, open: OpenLink, canOpenProject: boolean): ReactNode {
  if (typeof node === 'string') return node;
  const children = () => node.children.map((child, index) => inline(child, index, open, canOpenProject));
  const target = node.tag === 'link' ? node.target : undefined;
  const url = target?.kind === 'web' ? questionWebUrl(target.url) : null;
  const enabled = url || (target?.kind === 'project' && canOpenProject);
  if (Platform.OS === 'web' && target && enabled) {
    if (url) {
      return <a key={key} href={url} target="_blank" rel="noopener noreferrer" style={{ color: color.accent }}>
        {children()}
      </a>;
    }
    return <button
      key={key}
      type="button"
      onClick={() => open(target, label(node.children))}
      style={{
        appearance: 'none', border: 0, padding: 0, background: 'transparent', color: color.accent,
        font: 'inherit', cursor: 'pointer', textDecoration: 'underline',
      }}
    >{children()}</button>;
  }
  if (target && enabled) {
    return <Text
      key={key}
      accessibilityRole="link"
      style={{ color: color.accent, textDecorationLine: 'underline' }}
      onPress={() => open(target, label(node.children))}
    >{children()}</Text>;
  }
  return <Text key={key} style={inlineStyles[node.tag]}>{children()}</Text>;
}

export function QuestionMarkdown({ text, nodes, muted = false, open, canOpenProject }: Readonly<{
  text: string;
  nodes?: QuestionMarkdownNode[] | null;
  muted?: boolean;
  open: OpenLink;
  canOpenProject: boolean;
}>) {
  const style = muted ? s.small : s.text;
  const renderInline = (children: QuestionMarkdownNode[]) =>
    children.map((child, index) => inline(child, index, open, canOpenProject));

  function blocks(children: QuestionMarkdownNode[]): ReactNode {
    if (children.every(node => typeof node === 'string'
      || ['strong', 'emphasis', 'strike', 'code', 'link', 'plain'].includes(node.tag))) {
      return <Text selectable style={style}>{renderInline(children)}</Text>;
    }
    return children.map((node, index) => {
      if (typeof node === 'string') return <Text key={index} style={style} selectable>{node}</Text>;
      const items = node.children;
      switch (node.tag) {
        case 'heading':
          return <Text key={index} selectable accessibilityRole="header" style={s.subheading}>{renderInline(items)}</Text>;
        case 'code_block':
          return <Text key={index} selectable style={s.code}>{label(items)}</Text>;
        case 'quote':
          return <View key={index} style={{ borderStartWidth: 2, borderColor: color.line, paddingStart: 10, gap: 6 }}>{blocks(items)}</View>;
        case 'ordered_list':
        case 'list':
          return <View key={index} style={{ gap: 5 }}>
            {items.map((item, position) => <View key={position} style={{ flexDirection: 'row', gap: 8 }}>
              <Text style={style}>{node.tag === 'ordered_list' ? `${(node.start ?? 1) + position}.` : '•'}</Text>
              <View style={{ flex: 1, minWidth: 0 }}>{blocks(typeof item === 'string' ? [item] : item.children)}</View>
            </View>)}
          </View>;
        case 'table':
          return <View key={index} style={{ borderWidth: 1, borderColor: color.line }}>{blocks(items)}</View>;
        case 'row':
          return <View key={index} style={{ flexDirection: 'row', borderBottomWidth: 1, borderColor: color.line }}>
            {items.map((item, column) => <Text key={column} selectable style={[style, { flex: 1, minWidth: 0, padding: 6 }]}>
              {renderInline(typeof item === 'string' ? [item] : item.children)}
            </Text>)}
          </View>;
        case 'paragraph':
          return <Text key={index} style={style} selectable>{renderInline(items)}</Text>;
        default:
          return <Text key={index} style={style} selectable>{inline(node, index, open, canOpenProject)}</Text>;
      }
    });
  }

  if (!nodes?.length) return <Text selectable style={style}>{text}</Text>;
  return <View style={{ gap: 6 }}>{blocks(nodes)}</View>;
}
