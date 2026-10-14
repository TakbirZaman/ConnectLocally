plugins {
    id("com.android.application")
    id("kotlin-android")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

android {
    namespace = "com.example.localconnect"
    compileSdk = 35
    ndkVersion = "27.0.12077973"

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_11
        targetCompatibility = JavaVersion.VERSION_11
    }

    kotlinOptions {
        jvmTarget = JavaVersion.VERSION_11.toString()
    }

    defaultConfig {
        // Public install identity; Kotlin/JNI namespace is intentionally independent.
        applicationId = "io.github.takbirzaman.localconnect"
        // You can update the following values to match your application needs.
        // For more information, see: https://flutter.dev/to/review-gradle-config.
        minSdk = 26
        targetSdk = 35
        versionCode = flutter.versionCode
        versionName = flutter.versionName
        // Match Flutter's requested engines even when AGP retains stale JNI
        // intermediates from a preceding build with different architectures.
        val platforms = project.findProperty("target-platform")?.toString()?.split(",")
            ?: listOf("android-arm", "android-arm64", "android-x64")
        val abis = mapOf("android-arm" to "armeabi-v7a", "android-arm64" to "arm64-v8a",
            "android-x64" to "x86_64", "android-x86" to "x86")
        ndk {
            abiFilters.addAll(platforms.map { platform ->
                requireNotNull(abis[platform]) { "Unsupported Android target: $platform" }
            })
        }
    }

    val signingPath = System.getenv("LOCALCONNECT_KEYSTORE")
    signingConfigs {
        if (signingPath != null) {
            create("localconnectRelease") {
                storeFile = file(signingPath)
                storePassword = System.getenv("LOCALCONNECT_STORE_PASSWORD")
                keyAlias = System.getenv("LOCALCONNECT_KEY_ALIAS")
                keyPassword = System.getenv("LOCALCONNECT_KEY_PASSWORD")
            }
        }
    }
    buildTypes {
        release {
            // No silent debug signing for release. Supply your own protected key.
            if (signingPath != null) signingConfig = signingConfigs.getByName("localconnectRelease")
        }
    }

}

flutter {
    source = "../.."
}
