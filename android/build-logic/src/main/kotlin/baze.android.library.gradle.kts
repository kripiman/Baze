// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

import com.android.build.api.dsl.LibraryExtension
import org.jetbrains.kotlin.gradle.dsl.JvmTarget
import org.jetbrains.kotlin.gradle.dsl.KotlinAndroidProjectExtension

// See baze.android.application for why plugins are applied from the body.
pluginManager.apply("com.android.library")
pluginManager.apply("org.jetbrains.kotlin.android")

extensions.configure<LibraryExtension> {
    // :core:location -> cl.baze.core.location, which is the package of its sources.
    namespace = "cl.baze." + project.path.removePrefix(":").replace(":", ".")
    compileSdk = 35

    defaultConfig {
        minSdk = 26
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

extensions.configure<KotlinAndroidProjectExtension> {
    compilerOptions.jvmTarget.set(JvmTarget.JVM_17)
}
