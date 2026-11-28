package com.senzo.app.ui

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/**
 * The dark palette of the desktop application, in Compose form.
 *
 * It mirrors the desktop OLED Pitch Black preset exactly: the same true-black
 * page, one-step surfaces, violet accent and white text. Keeping the palette
 * identical means a title looks the same whether it is opened on the desktop
 * or on the phone, and the brand does not drift between platforms.
 */
val Background = Color(0xFF000000)
val BackgroundElevated = Color(0xFF121212)
val Surface = Color(0xFF121212)
val SurfaceHigh = Color(0xFF181818)
val Accent = Color(0xFFA855F7)
val AccentDeep = Color(0xFF7E22CE)
val OnAccent = Color(0xFFFFFFFF)
val TextPrimary = Color(0xFFFFFFFF)
val TextMuted = Color(0xFFB3B3B3)
val TextDim = Color(0xFF808080)
val Gold = Color(0xFFFFC857)
val Outline = Color(0xFF282828)

private val SenzoColors = darkColorScheme(
    primary = Accent,
    onPrimary = OnAccent,
    primaryContainer = Accent.copy(alpha = 0.16f),
    onPrimaryContainer = TextPrimary,
    secondary = Gold,
    onSecondary = Color.Black,
    background = Background,
    onBackground = TextPrimary,
    surface = Surface,
    onSurface = TextPrimary,
    surfaceVariant = SurfaceHigh,
    onSurfaceVariant = TextMuted,
    surfaceContainer = BackgroundElevated,
    outline = Outline,
    outlineVariant = Outline,
    error = Color(0xFFFF6B6B),
)

private val SenzoTypography = Typography(
    displaySmall = TextStyle(
        fontSize = 34.sp,
        lineHeight = 38.sp,
        fontWeight = FontWeight.Bold,
        letterSpacing = (-0.5).sp,
    ),
    headlineMedium = TextStyle(fontSize = 26.sp, fontWeight = FontWeight.SemiBold),
    headlineSmall = TextStyle(fontSize = 22.sp, fontWeight = FontWeight.SemiBold),
    titleLarge = TextStyle(fontSize = 20.sp, fontWeight = FontWeight.SemiBold),
    titleMedium = TextStyle(fontSize = 16.sp, fontWeight = FontWeight.Medium),
    titleSmall = TextStyle(fontSize = 14.sp, fontWeight = FontWeight.Medium),
    bodyLarge = TextStyle(fontSize = 15.sp, lineHeight = 22.sp),
    bodyMedium = TextStyle(fontSize = 14.sp, lineHeight = 20.sp),
    bodySmall = TextStyle(fontSize = 12.sp, lineHeight = 16.sp),
    labelLarge = TextStyle(fontSize = 14.sp, fontWeight = FontWeight.SemiBold),
    labelMedium = TextStyle(fontSize = 12.sp, fontWeight = FontWeight.Medium),
    labelSmall = TextStyle(fontSize = 10.sp, fontWeight = FontWeight.Medium),
)

private val SenzoShapes = Shapes(
    extraSmall = RoundedCornerShape(8.dp),
    small = RoundedCornerShape(12.dp),
    medium = RoundedCornerShape(16.dp),
    large = RoundedCornerShape(22.dp),
    extraLarge = RoundedCornerShape(28.dp),
)

/** Applies the palette, type scale and rounded shapes the app is drawn with. */
@Composable
fun SenzoTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = SenzoColors,
        typography = SenzoTypography,
        shapes = SenzoShapes,
        content = content,
    )
}

/** The wordmark used in headers, drawn instead of shipped as an image. */
val SenzoWordmark = TextStyle(
    fontFamily = FontFamily.SansSerif,
    fontWeight = FontWeight.Black,
    fontSize = 22.sp,
    letterSpacing = 4.sp,
)