// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

import org.jetbrains.kotlin.gradle.dsl.JvmTarget
import org.jetbrains.kotlin.gradle.dsl.KotlinJvmProjectExtension

// Pure Kotlin modules (AGENTS §6.2): no Android, no java.* in the domain. See baze.android.application
// for why plugins are applied from the body.
pluginManager.apply("org.jetbrains.kotlin.jvm")
pluginManager.apply("org.jetbrains.kotlin.plugin.serialization")

extensions.configure<JavaPluginExtension> {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

extensions.configure<KotlinJvmProjectExtension> {
    compilerOptions.jvmTarget.set(JvmTarget.JVM_17)
}

// The rule is easy to break with one auto-import, so it is enforced by the build, not by review.
val checkPurity = tasks.register("checkPurity") {
    group = "verification"
    description = "Fails if a pure Kotlin module imports java.*, javax.*, android.* or androidx.*"
    val sources = fileTree("src") { include("**/*.kt") }
    val moduleDir = projectDir
    inputs.files(sources)
    doLast {
        val forbidden = Regex("""^\s*import\s+(java|javax|android|androidx)\.""")
        val offenders = sources.files.sorted().flatMap { file ->
            file.readLines().mapIndexedNotNull { index, line ->
                if (forbidden.containsMatchIn(line)) {
                    "${file.relativeTo(moduleDir)}:${index + 1}: ${line.trim()}"
                } else {
                    null
                }
            }
        }
        if (offenders.isNotEmpty()) {
            throw GradleException(
                "Pure Kotlin modules must not import platform APIs:\n" + offenders.joinToString("\n"),
            )
        }
    }
}

tasks.named("check") { dependsOn(checkPurity) }
