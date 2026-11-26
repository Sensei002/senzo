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

# libtorrent4j reaches its Java surface from native code, so the SWIG names
# have to survive R8 untouched.
-keep class org.libtorrent4j.** { *; }
-keep class org.libtorrent4j.swig.** { *; }
-dontwarn org.libtorrent4j.**

# QuickJS calls into its Java surface from JNI, and the wrapper hands JSObject
# callbacks back to native code, so every name in the wrapper must survive.
-keep class com.whl.quickjs.** { *; }
-keepclasseswithmembernames class * {
    native <methods>;
}
-dontwarn com.whl.quickjs.**

# The FFmpeg audio decoder is reached by name from native code and replaces
# the platform decoders for AC3, EAC3, DTS and TrueHD.
-keep class androidx.media3.decoder.ffmpeg.** { *; }
-keepclasseswithmembernames class androidx.media3.decoder.ffmpeg.** {
    native <methods>;
}

# libmpv calls back into is.xyz.mpv.MPV from JNI by name, and the native
# player loads its Java observers reflectively.
-keep class is.xyz.mpv.** { *; }
-keepclasseswithmembernames class is.xyz.mpv.** {
    native <methods>;
}
-dontwarn is.xyz.mpv.**

# Media3 and Guava reference desktop-only entry points.
-dontwarn androidx.media3.**
-dontwarn com.google.common.**
-dontwarn com.google.errorprone.annotations.**
-dontwarn javax.annotation.**

# Keep line numbers so a release stack trace points at a real source line.
-keepattributes SourceFile,LineNumberTable
-renamesourcefileattribute SourceFile
