import type { ReactNode, Ref } from 'react';
import {
  Pressable,
  ScrollView,
  Text,
  TextInput,
  View,
  type KeyboardTypeOptions,
  type ScrollViewInstance,
  type ScrollViewProps,
  type TextInputInstance,
} from 'react-native';
import { createContext, forwardRef, useContext, useState } from 'react';
import { color, styles as s } from './theme';

export const PageNotices = createContext<ReactNode>(null);
export const PageScroll = forwardRef<ScrollViewInstance, ScrollViewProps>(function PageScroll(
  { children, ...props },
  ref,
) {
  const notices = useContext(PageNotices);
  return (
    <ScrollView ref={ref} keyboardDismissMode="on-drag" {...props}>
      {notices}
      {children}
    </ScrollView>
  );
});

export function Button({
  children,
  onPress,
  disabled = false,
  primary = false,
  quiet = false,
  danger = false,
  testID,
}: Readonly<{
  children: string;
  onPress: () => void;
  disabled?: boolean;
  primary?: boolean;
  quiet?: boolean;
  danger?: boolean;
  testID?: string;
}>) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityState={{ disabled }}
      disabled={disabled}
      onPress={onPress}
      testID={testID}
      style={({ pressed }) => [
        s.button,
        primary && s.primary,
        quiet && s.quiet,
        disabled && s.disabled,
        pressed && s.pressed,
      ]}
    >
      <Text style={[s.buttonText, primary && s.primaryText, danger && s.dangerText]}>
        {children}
      </Text>
    </Pressable>
  );
}
export function Field({
  label,
  value,
  onChange,
  placeholder,
  secret = false,
  multiline = false,
  maxLength,
  keyboardType,
  testID,
  onFocus,
  onBlur,
  inputRef,
  editable = true,
}: Readonly<{
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  secret?: boolean;
  multiline?: boolean;
  maxLength?: number;
  keyboardType?: KeyboardTypeOptions;
  testID?: string;
  onFocus?: () => void;
  onBlur?: () => void;
  inputRef?: Ref<TextInputInstance>;
  editable?: boolean;
}>) {
  const [focused, focus] = useState(false);
  return (
    <View>
      <Text style={s.label}>{label}</Text>
      <TextInput
        ref={inputRef}
        accessibilityLabel={label}
        value={value}
        onChangeText={onChange}
        editable={editable}
        placeholder={placeholder}
        placeholderTextColor={color.muted}
        secureTextEntry={secret}
        autoCapitalize="none"
        autoCorrect={false}
        autoComplete="off"
        multiline={multiline}
        maxLength={maxLength}
        // Android combines URL and password variations into a visible-password input.
        keyboardType={secret ? 'default' : keyboardType}
        onFocus={() => {
          focus(true);
          onFocus?.();
        }}
        onBlur={() => {
          focus(false);
          onBlur?.();
        }}
        testID={testID}
        style={[s.input, multiline && s.draft, focused && s.focused]}
      />
    </View>
  );
}
export function Notice({
  children,
  error = false,
}: Readonly<{ children: ReactNode; error?: boolean }>) {
  return (
    <Text
      accessibilityRole={error ? 'alert' : 'text'}
      accessibilityLiveRegion="polite"
      style={[s.text, error && s.errorText]}
    >
      {children}
    </Text>
  );
}
export function StateBadge({ value }: Readonly<{ value: string }>) {
  return (
    <View style={s.badge}>
      <Text
        style={[
          s.small,
          {
            color:
              value === 'completed'
                ? color.green
                : value.includes('approval') || value === 'needs_answer'
                  ? color.accent
                  : color.muted,
          },
        ]}
      >
        {value.replaceAll('_', ' ')}
      </Text>
    </View>
  );
}
