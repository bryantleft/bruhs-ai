import { getDocumentAsync } from 'expo-document-picker';
import { File } from 'expo-file-system';
import { launchImageLibraryAsync } from 'expo-image-picker';
import { Platform } from 'react-native';

export type PickedQuestionFile = Readonly<{ name: string; body: Blob }>;

export async function pickQuestionFiles(kind: 'files' | 'photos'): Promise<PickedQuestionFile[]> {
  if (kind === 'photos') {
    const result = await launchImageLibraryAsync({
      mediaTypes: ['images'],
      allowsMultipleSelection: true,
    });
    return (result.assets ?? []).map(asset => pickedFile(
      asset.fileName || asset.file?.name || asset.uri.split('/').pop() || 'photo.jpg',
      asset.uri,
      asset.file,
    ));
  }
  const result = await getDocumentAsync({
    multiple: true,
    copyToCacheDirectory: true,
    base64: false,
  });
  return (result.assets ?? []).map(asset => pickedFile(asset.name, asset.uri, asset.file));
}

function pickedFile(name: string, uri: string, file?: Blob): PickedQuestionFile {
  if (Platform.OS === 'web') {
    if (!file) throw new Error('The browser did not return the selected file.');
    return { name, body: file };
  }
  return { name, body: new File(uri) };
}
