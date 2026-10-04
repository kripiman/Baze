# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later

# Rules for the release build (R8). The libraries below do not ship their own consumer rules, or
# not enough of them.

# kotlinx.serialization looks the generated serializers up by name.
-keepattributes *Annotation*, InnerClasses
-dontnote kotlinx.serialization.AnnotationsKt
-keepclassmembers class kotlinx.serialization.json.** {
    *** Companion;
}
-keepclasseswithmembers class kotlinx.serialization.json.** {
    kotlinx.serialization.KSerializer serializer(...);
}
-keep,includedescriptorclasses class cl.baze.core.model.**$$serializer { *; }
-keepclassmembers class cl.baze.core.model.** {
    *** Companion;
}
-keepclasseswithmembers class cl.baze.core.model.** {
    kotlinx.serialization.KSerializer serializer(...);
}

# Ktor: the SLF4J binding and the JVM management beans are optional and absent on Android.
-dontwarn org.slf4j.**
-dontwarn java.lang.management.**
-dontwarn javax.management.**
