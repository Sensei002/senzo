# Rules for the release build. Everything the app reaches through reflection
# rather than a direct call has to be named here, because R8 cannot see it.

# kotlinx.serialization resolves serializers by name at runtime, so the
# generated $$serializer classes and their companion members must survive.
-keepattributes *Annotation*, InnerClasses, Signature, RuntimeVisibleAnnotations, AnnotationDefault
-dontnote kotlinx.serialization.**
-keepclassmembers class kotlinx.serialization.json.** {
    *** Companion;
}
-keepclasseswithmembers class kotlinx.serialization.json.** {
    kotlinx.serialization.KSerializer serializer(...);
}
-keep,includedescriptorclasses class com.senzo.app.**$$serializer { *; }
-keepclassmembers class com.senzo.app.** {
    *** Companion;
}
-keepclasseswithmembers class com.senzo.app.** {
    kotlinx.serialization.KSerializer serializer(...);
}

# OkHttp looks for optional platform integrations that Android does not ship.
-dontwarn org.conscrypt.**
-dontwarn org.bouncycastle.**
-dontwarn org.openjsse.**
-dontwarn okhttp3.internal.platform.**
-dontwarn okio.**

# Media3 and Guava reference desktop-only entry points.
-dontwarn androidx.media3.**
-dontwarn com.google.common.**
-dontwarn com.google.errorprone.annotations.**
-dontwarn javax.annotation.**

# Keep line numbers so a release stack trace points at a real source line.
-keepattributes SourceFile,LineNumberTable
-renamesourcefileattribute SourceFile
