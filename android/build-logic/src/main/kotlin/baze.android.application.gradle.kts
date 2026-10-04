// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

import com.android.build.api.dsl.ApplicationExtension
import org.jetbrains.kotlin.gradle.dsl.JvmTarget
import org.jetbrains.kotlin.gradle.dsl.KotlinAndroidProjectExtension
import java.util.Properties

// The Android and Kotlin plugins are applied by id from the body, not from a `plugins {}` block: AGP
// is only a compileOnly dependency of build-logic (see build.gradle.kts), so the root build loads
// it once and every module shares that single copy. A `plugins {}` block would need it on
// build-logic's own classpath, which is what "plugin was not found" in CI was about.
pluginManager.apply("com.android.application")
pluginManager.apply("org.jetbrains.kotlin.android")
pluginManager.apply("org.jetbrains.kotlin.plugin.compose")

// Release signing comes from a gitignored keystore.properties; without it the release APK is unsigned.
val keystoreProperties = Properties().apply {
    val file = rootProject.file("keystore.properties")
    if (file.exists()) file.inputStream().use { load(it) }
}

extensions.configure<ApplicationExtension> {
    namespace = "cl.baze.app"
    compileSdk = 35

    defaultConfig {
        applicationId = "cl.baze.app"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    signingConfigs {
        if (keystoreProperties.containsKey("storeFile")) {
            create("release") {
                storeFile = rootProject.file(keystoreProperties.getProperty("storeFile"))
                storePassword = keystoreProperties.getProperty("storePassword")
                keyAlias = keystoreProperties.getProperty("keyAlias")
                keyPassword = keystoreProperties.getProperty("keyPassword")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
            signingConfigs.findByName("release")?.let { signingConfig = it }
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
    }
}

extensions.configure<KotlinAndroidProjectExtension> {
    compilerOptions.jvmTarget.set(JvmTarget.JVM_17)
}
