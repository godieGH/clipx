/**
 * Android application module for Clipx.
 *
 * This Gradle script configures the app namespace, SDK targets, and the custom
 * Rust-to-Android FFI build steps that generate and package the native bridge.
 */
plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

val version = rootProject
    .file("../../VERSION")
    .readText()
    .trim()

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
        versionName = version

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

val targetAbis = (findProperty("clipxAbis") as String? ?: "arm64-v8a")
    .split(",")
    .map { it.trim() }

tasks.register<Exec>("buildRustAndroidFfiBridge") {
    group = "custom"
    description = "This task builds the rust-mobile-ffi-bridge for android via uniffi."

    inputs.dir("../../native/rust-mobile-ffi-bridge/src")
    inputs.file("../../native/rust-mobile-ffi-bridge/Cargo.toml")
    inputs.property("targetAbis", targetAbis)
    outputs.dir("src/main/jniLibs")

    workingDir("../../native")
    val abiFlags = targetAbis.flatMap { listOf("-t", it) }
    commandLine(
        listOf("cargo", "ndk") + abiFlags +
                listOf("-o", "../android/app/src/main/jniLibs", "build", "--package", "rust-mobile-ffi-bridge", "--release")
    )
}

tasks.register<Exec>("generateBindingsForFfiBridge") {
    group = "custom"
    description = "This task generates ffi bindings to call rust from android kotlin/java"

    dependsOn("buildRustAndroidFfiBridge")

    inputs.file("../../native/target/aarch64-linux-android/release/librust_mobile_ffi_bridge.so")
    inputs.file("../../native/rust-mobile-ffi-bridge/uniffi.toml")
    outputs.dir("src/main/java/com/godiegh/clipx/ffi")

    workingDir("../../native")
    commandLine(
        "cargo", "run", "--bin", "uniffi-bindgen",
        "generate",
        "--library", "target/aarch64-linux-android/release/librust_mobile_ffi_bridge.so",
        "--language", "kotlin",
        "--out-dir", "../android/app/src/main/java"
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
    implementation(libs.jna) {
        artifact {
            type = "aar"
        }
    }
    implementation(libs.kotlinx.coroutines.core)

    testImplementation(libs.junit)
    androidTestImplementation(platform(libs.androidx.compose.bom))
    androidTestImplementation(libs.androidx.compose.ui.test.junit4)
    androidTestImplementation(libs.androidx.espresso.core)
    androidTestImplementation(libs.androidx.junit)
    debugImplementation(libs.androidx.compose.ui.test.manifest)
    debugImplementation(libs.androidx.compose.ui.tooling)
}