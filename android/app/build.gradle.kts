import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

// ключ подписи лежит в android/keystore.properties (в git не попадает), см. android/README.md
val keystoreProps = Properties().apply {
    val f = rootProject.file("keystore.properties")
    if (f.exists()) f.inputStream().use { load(it) }
}

android {
    namespace = "ru.catpaws.nebulaclient"
    compileSdk = 35

    defaultConfig {
        applicationId = "ru.catpaws.nebulaclient"
        minSdk = 26
        targetSdk = 35
        // версия и лента обновлений переопределяются для проверки: -PappVersionCode=... -PappVersionName=... -PupdateFeed=...
        versionCode = (findProperty("appVersionCode") as String?)?.toInt() ?: 5
        versionName = (findProperty("appVersionName") as String?) ?: "0.5.2-android5"
        val feed = (findProperty("updateFeed") as String?) ?: "https://api.github.com/repos/Arseniy07955/nebula-client/releases?per_page=30"
        buildConfigField("String", "UPDATE_FEED", "\"$feed\"")
        manifestPlaceholders["cleartext"] = if (feed.startsWith("http://")) "true" else "false"
        ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a") }
    }

    signingConfigs {
        if (keystoreProps.isNotEmpty()) {
            create("release") {
                storeFile = rootProject.file(keystoreProps.getProperty("storeFile"))
                storePassword = keystoreProps.getProperty("storePassword")
                keyAlias = keystoreProps.getProperty("storeAlias")
                keyPassword = keystoreProps.getProperty("keyPassword")
                enableV1Signing = true
                enableV2Signing = true
                enableV3Signing = true
            }
        }
    }
    buildTypes {
        release {
            isMinifyEnabled = false
            signingConfigs.findByName("release")?.let { signingConfig = it }
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    buildFeatures {
        compose = true
        buildConfig = true
    }
    packaging { jniLibs.useLegacyPackaging = true }
}

dependencies {
    implementation(files("libs/nebulacore.aar"))
    implementation(platform("androidx.compose:compose-bom:2024.12.01"))
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")
    implementation("androidx.work:work-runtime-ktx:2.10.0")
    implementation("androidx.core:core-ktx:1.15.0")
}
