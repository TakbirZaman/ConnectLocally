import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:localconnect/desktop_preferences.dart';

void main() {
  late Directory root;
  late DesktopPreferences preferences;
  setUp(() async {
    root = await Directory.systemTemp.createTemp('localconnect-preferences-');
    preferences = DesktopPreferences(File('${root.path}/support/preferences.json'));
  });
  tearDown(() async => root.delete(recursive: true));

  test('first start creates default; restart retains chosen folder', () async {
    final defaultPath = '${root.path}/Downloads/LocalConnect';
    expect(await preferences.resolveReceiveFolder(defaultPath), defaultPath);
    final selected = await Directory('${root.path}/My received files').create();
    await preferences.saveReceiveFolder(selected.path);
    final restarted = DesktopPreferences(preferences.file);
    expect(await restarted.resolveReceiveFolder(defaultPath), selected.path);
  });

  test('missing saved folder does not silently redirect or recreate it', () async {
    final selected = await Directory('${root.path}/removable').create();
    await preferences.saveReceiveFolder(selected.path);
    await selected.delete();
    await expectLater(
      preferences.resolveReceiveFolder('${root.path}/fallback'),
      throwsA(isA<FileSystemException>()),
    );
    expect(await selected.exists(), isFalse);
    expect(await Directory('${root.path}/fallback').exists(), isFalse);
  });

  test('invalid preferences recover; invalid selection preserves saved folder', () async {
    await preferences.file.parent.create(recursive: true);
    await preferences.file.writeAsString('{broken');
    final path = await preferences.resolveReceiveFolder('${root.path}/default');
    await expectLater(preferences.saveReceiveFolder(''), throwsA(isA<FileSystemException>()));
    await expectLater(preferences.saveReceiveFolder('${root.path}/missing'), throwsA(isA<FileSystemException>()));
    expect(await preferences.receiveFolder(), path);
  });
}
