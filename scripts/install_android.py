#!/usr/bin/env python3
"""Run AFTER flutter create --platforms=windows,linux,android .
Only merges this project's native files/permissions, never removes other grants.
"""
from pathlib import Path
import shutil
import xml.etree.ElementTree as ET
root=Path(__file__).resolve().parents[1]
app=root/'android/app'
assert (app/'src/main/AndroidManifest.xml').exists(), 'Run flutter create first'
dest=app/'src/main/kotlin/com/example/localconnect'
dest.mkdir(parents=True,exist_ok=True)
for file in (root/'android-native').glob('*.kt'):
    shutil.copy2(file,dest/file.name)
xml=app/'src/main/AndroidManifest.xml'
ET.register_namespace('android','http://schemas.android.com/apk/res/android')
ns='{http://schemas.android.com/apk/res/android}'
tree=ET.parse(xml);manifest=tree.getroot()
add=ET.parse(root/'android-native/manifest-additions.xml').getroot()
for permission in add.findall('uses-permission'):
    if not any(x.get(ns+'name')==permission.get(ns+'name') for x in manifest.findall('uses-permission')):
        manifest.insert(0,permission)
application=manifest.find('application');assert application is not None
application.set(ns+'allowBackup','false')
application.set(ns+'fullBackupContent','false')
for service in add.find('application').findall('service'):
    if not any(x.get(ns+'name')==service.get(ns+'name') for x in application.findall('service')):
        application.append(service)
    else:
        old=next(x for x in application.findall('service') if x.get(ns+'name')==service.get(ns+'name'))
        application.remove(old);application.append(service)
tree.write(xml,encoding='utf-8',xml_declaration=True)
res=app/'src/main/res/xml';res.mkdir(parents=True,exist_ok=True)
shutil.copy2(root/'android-native/accessibility_service.xml',res/'accessibility_service.xml')
gradle=app/'build.gradle.kts'
assert gradle.exists(), 'Use the pinned Flutter 3.29.3 Kotlin DSL template'
s=gradle.read_text().replace('compileSdk = flutter.compileSdkVersion','compileSdk = 35').replace('targetSdk = flutter.targetSdkVersion','targetSdk = 35').replace('minSdk = flutter.minSdkVersion','minSdk = 26').replace('ndkVersion = flutter.ndkVersion','ndkVersion = "27.0.12077973"')
if 'val platforms = project.findProperty("target-platform")' not in s:
    anchor='        versionName = flutter.versionName'
    assert anchor in s, 'Pinned Flutter defaultConfig template missing'
    s=s.replace(anchor,anchor+'''
        // Match Flutter's requested engines even with stale JNI intermediates.
        val platforms = project.findProperty("target-platform")?.toString()?.split(",")
            ?: listOf("android-arm", "android-arm64", "android-x64")
        val abis = mapOf("android-arm" to "armeabi-v7a", "android-arm64" to "arm64-v8a",
            "android-x64" to "x86_64", "android-x86" to "x86")
        ndk {
            abiFilters.addAll(platforms.map { platform ->
                requireNotNull(abis[platform]) { "Unsupported Android target: $platform" }
            })
        }
''')
gradle.write_text(s)
settings=root/'android/settings.gradle.kts'
text=settings.read_text()
import re
text=re.sub(r'(id\("com.android.application"\) version )"[^"]+"',r'\1"8.7.3"',text)
text=re.sub(r'(id\("org.jetbrains.kotlin.android"\) version )"[^"]+"',r'\1"2.1.0"',text)
settings.write_text(text)
print('Installed Android source overlays; API 35 target, minimum API 26. Native build still required.')
