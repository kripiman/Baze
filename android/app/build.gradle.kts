// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

plugins {
    id("baze.android.application")
    alias(libs.plugins.licensee)
}

// Only licenses compatible with distributing the app under AGPL-3.0-or-later (and through F-Droid)
// may ship. A new dependency under any other license fails `checkLicenses` until it is reviewed.
licensee {
    allow("Apache-2.0")
    allow("MIT")
    allow("BSD-2-Clause")
    allow("BSD-3-Clause")
}

// AGENTS §6.1: `./gradlew checkLicenses`. It audits what the release APK really contains.
tasks.register("checkLicenses") {
    group = "verification"
    description = "Fails if the release APK ships a dependency whose license is not approved"
    dependsOn("licenseeRelease")
}

dependencies {
    implementation(project(":core:model"))
    implementation(project(":core:domain"))
    implementation(project(":core:network"))
    implementation(project(":core:location"))

    implementation(project(":feature:map"))
    implementation(project(":feature:search"))
    implementation(project(":feature:navigation"))
    implementation(project(":feature:reports"))

    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation(libs.androidx.activity.compose)

    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.material.icons)
    implementation(libs.androidx.compose.ui.tooling.preview)
    debugImplementation(libs.androidx.compose.ui.tooling)

    implementation(libs.koin.android)
    implementation(libs.koin.androidx.compose)

    testImplementation(libs.test.junit)
    androidTestImplementation(libs.test.androidx.junit)
    androidTestImplementation(libs.test.androidx.espresso)
}
