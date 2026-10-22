import 'dart:convert';
import 'dart:io';

/// Local UI preferences only. Identities and transfer recovery remain in Rust.
class DesktopPreferences {
  DesktopPreferences(this.file);
  final File file;

  Future<String?> receiveFolder() async {
    if (!await file.exists()) return null;
    try {
      final value = jsonDecode(await file.readAsString());
      if (value is! Map || value['receiveFolder'] is! String) return null;
      final path = value['receiveFolder'] as String;
      return path.trim().isEmpty ? null : path;
    } on FormatException {
      return null;
    }
  }

  Future<void> saveReceiveFolder(String path) async {
    if (path.trim().isEmpty || !await Directory(path).exists()) {
      throw FileSystemException('Choose an existing receive folder', path);
    }
    await file.parent.create(recursive: true);
    await file.writeAsString(jsonEncode({'receiveFolder': path}), flush: true);
  }

  Future<String> resolveReceiveFolder(String defaultPath) async {
    final saved = await receiveFolder();
    if (saved != null) {
      // Do not recreate a missing removable/network folder at a different location.
      if (!await Directory(saved).exists()) {
        throw FileSystemException(
          'Saved receive folder is unavailable. Reconnect the drive or choose another folder before starting.',
          saved,
        );
      }
      return saved;
    }
    final folder = await Directory(defaultPath).create(recursive: true);
    await saveReceiveFolder(folder.path);
    return folder.path;
  }
}
