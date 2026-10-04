// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

plugins {
    `kotlin-dsl`
}

// compileOnly on purpose: the root build loads these plugins (`apply false`) and the convention
// plugins run against that single copy. Putting them on build-logic's runtime classpath too would load
// the Android Gradle plugin twice, in classloaders that cannot see each other.
dependencies {
    compileOnly(libs.android.gradlePlugin)
    compileOnly(libs.kotlin.gradlePlugin)
}
