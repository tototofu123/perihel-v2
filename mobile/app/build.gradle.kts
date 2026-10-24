plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "com.tototofu.perihel"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.tototofu.perihel"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        versionName = "1.0"
        // optional: build with your server baked in:
        //   ./gradlew assembleDebug -PPERIHEL_URL=... -PPERIHEL_TOKEN=...
        buildConfigField("String", "DEFAULT_URL", "\"${project.findProperty("PERIHEL_URL") ?: ""}\"")
        buildConfigField("String", "DEFAULT_TOKEN", "\"${project.findProperty("PERIHEL_TOKEN") ?: ""}\"")
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }
    buildFeatures {
        compose = true
        buildConfig = true
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
    packaging {
        resources.excludes += "META-INF/*"
    }
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2024.09.03")
    implementation(composeBom)
    implementation("androidx.activity:activity-compose:1.9.2")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.6")
    debugImplementation("androidx.compose.ui:ui-tooling")
}
