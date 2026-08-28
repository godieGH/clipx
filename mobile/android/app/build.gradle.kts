plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "com.godiegh.clipx"
    compileSdk {
        version = release(37)
    }

    defaultConfig {
        applicationId = "com.godiegh.clipx"
        minSdk = 29
        targetSdk = 37
        versionCode = 1
        versionName = "1.0"

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    buildTypes {
        release {
            optimization {
                enable = false
            }
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_11
        targetCompatibility = JavaVersion.VERSION_11
    }
    buildFeatures {
        compose = true
    }
}

tasks.register<Exec>("buildRustAndroidFfiBridge") {
    group = "custom"
    description = "This task builds the rust-android-ffi-bridge via uniffi."

    workingDir("../native/rust-android-ffi-bridge")
    commandLine("cargo", "ndk", "-t", "arm64-v8a", "-t", "x86_64", "-o", "../../app/src/main/jniLibs", "build", "--release")
}

tasks.register<Exec>("generateBindingsForFfiBridge") {
    group = "custom"
    description = "This task generates ffi bindings to call rust from android kotlin/java"

    dependsOn("buildRustAndroidFfiBridge")
    workingDir("../native/rust-android-ffi-bridge")
    commandLine(
        "cargo", "run", "--bin", "uniffi-bindgen",
        "generate",
        "--library", "target/aarch64-linux-android/release/librust_android_ffi_bridge.so",
        "--language", "kotlin",
        "--out-dir", "../../app/src/main/java/com/godiegh/clipx"
    )
}

tasks.named("preBuild") {
    dependsOn("generateBindingsForFfiBridge")
}

dependencies {
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.graphics)
    implementation(libs.androidx.compose.ui.tooling.preview)
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.lifecycle.runtime.ktx)

    implementation(libs.androidx.compose.material.icons.extended)
    implementation(libs.androidx.navigation.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation("net.java.dev.jna:jna:5.13.0@aar")
    implementation(libs.kotlinx.coroutines.core)

    testImplementation(libs.junit)
    androidTestImplementation(platform(libs.androidx.compose.bom))
    androidTestImplementation(libs.androidx.compose.ui.test.junit4)
    androidTestImplementation(libs.androidx.espresso.core)
    androidTestImplementation(libs.androidx.junit)
    debugImplementation(libs.androidx.compose.ui.test.manifest)
    debugImplementation(libs.androidx.compose.ui.tooling)
}