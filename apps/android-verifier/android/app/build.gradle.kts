plugins {
    id("com.android.application")
    id("dev.flutter.flutter-gradle-plugin")
}
android {
    namespace = "io.github.naza3.nexa.verifier"
    compileSdk = 36
    ndkVersion = "30.0.16248370"
    compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
    defaultConfig {
        applicationId = "io.github.naza3.nexa.verifier"
        minSdk = 28
        targetSdk = 36
        versionCode = flutter.versionCode
        versionName = flutter.versionName
        ndk { abiFilters += "arm64-v8a" }
    }
    buildTypes {
        release {
            // Internal research build only, deliberately not a production key.
            signingConfig = signingConfigs.getByName("debug")
            isMinifyEnabled = false
            isShrinkResources = false
        }
    }
    packaging { jniLibs { useLegacyPackaging = false } }
}
kotlin { compilerOptions { jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17 } }
flutter { source = "../.." }
