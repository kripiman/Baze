// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

plugins {
    id("baze.android.feature")
}

dependencies {
    implementation(project(":core:model"))
    implementation(project(":core:location"))

    // MapLibre Native Android (≥ 11.7.0)
    implementation(libs.maplibre.android)

    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.ui.tooling.preview)

    implementation(libs.koin.androidx.compose)

    testImplementation(libs.test.junit)
}
