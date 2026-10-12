package com.audiorouter.app;

import android.Manifest;
import android.app.Activity;
import android.app.AlertDialog;
import android.bluetooth.BluetoothAdapter;
import android.bluetooth.BluetoothDevice;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.ActivityNotFoundException;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.res.ColorStateList;
import android.content.pm.PackageManager;
import android.graphics.Color;
import android.graphics.Typeface;
import android.graphics.drawable.ColorDrawable;
import android.graphics.drawable.GradientDrawable;
import android.media.projection.MediaProjectionManager;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.view.Window;
import android.view.WindowInsets;
import android.view.animation.DecelerateInterpolator;
import android.view.animation.LinearInterpolator;
import android.animation.Animator;
import android.animation.AnimatorListenerAdapter;
import android.animation.ObjectAnimator;
import android.animation.ValueAnimator;
import android.widget.Button;
import android.widget.FrameLayout;
import android.widget.EditText;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.PopupWindow;
import android.widget.ScrollView;
import android.widget.SeekBar;
import android.widget.Switch;
import android.widget.TextView;
import android.widget.Toast;
import android.text.TextUtils;
import android.text.Editable;
import android.text.TextWatcher;

import java.util.ArrayList;
import java.util.List;
import java.util.Set;
import java.util.function.IntConsumer;

public final class MainActivity extends Activity {
    private static final String PREFS_NAME = "audiorouter-settings";
    private static final String PREF_LAN_LISTEN_ENABLED = "lan_listen_enabled";
    private static final String PREF_BLUETOOTH_LISTEN_ENABLED = "bluetooth_listen_enabled";
    private static final String PREF_SYSTEM_VOLUME_ENABLED = "system_volume_enabled";
    private static final String PREF_THEME_MODE = "theme_mode";
    private static final String PREF_PRECONNECT_CONTROLS = "preconnect_controls";
    private static final String PREF_SHOW_IP_ADDRESS = "show_ip_address";
    private static final String PREF_RESTORE_CONNECTION_STATE = "restore_connection_state";
    private static final String PREF_AUTO_CONNECT = "auto_connect";
    private static final String PREF_AUTO_CONNECT_ATTEMPTS = "auto_connect_attempts";
    private static final String PREF_AUTO_CONNECT_INTERVAL_SECONDS = "auto_connect_interval_seconds";
    private static final String PREF_CUSTOM_AUDIO_BUFFER_MS = "custom_audio_buffer_ms";
    private static final String PREF_CUSTOM_AUDIO_BUFFER_SELECTED = "custom_audio_buffer_selected";
    private static final String THEME_SYSTEM = "system";
    private static final String THEME_LIGHT = "light";
    private static final String THEME_DARK = "dark";
    private static final int REQUEST_CAPTURE = 200;
    private static final int REQUEST_PERMISSIONS = 201;
    private static final int REQUEST_BLUETOOTH = 202;
    private static final int REQUEST_ENABLE_BLUETOOTH = 203;

    private int COLOR_BACKGROUND = Color.rgb(13, 20, 19);
    private int COLOR_SURFACE = Color.rgb(22, 34, 31);
    private int COLOR_SURFACE_ALT = Color.rgb(31, 48, 44);
    private int COLOR_LINE = Color.rgb(56, 80, 73);
    private int COLOR_TEXT = Color.rgb(237, 245, 240);
    private int COLOR_MUTED = Color.rgb(157, 181, 172);
    private int COLOR_MINT = Color.rgb(143, 221, 195);
    private int COLOR_AMBER = Color.rgb(242, 174, 112);

    private final Handler handler = new Handler(Looper.getMainLooper());
    private FrameLayout pageHost;
    private ScrollView homePage;
    private ScrollView overviewPage;
    private ScrollView currentPage;
    private int homeScrollY;
    private boolean capturePending;
    private boolean bluetoothPermissionPending;
    private boolean updatingTransportSwitches;
    private Runnable customBufferSave;
    private final List<DiscoverySession.Device> lanDevices = new ArrayList<>();
    private final List<BluetoothDevice> bluetoothDevices = new ArrayList<>();
    private String pendingLanIdentity = "";
    private String pendingBluetoothIdentity = "";
    private final Runnable bluetoothRefresh = this::refreshBluetoothDevicesNow;
    private final Runnable lanConnectionTimeout = () -> finishPendingConnection(true);
    private final Runnable bluetoothConnectionTimeout = () -> finishPendingConnection(false);
    private final Runnable captureStartTimeout = () -> {
        if (capturePending) {
            cancelCaptureRequest();
            Toast.makeText(this, "音频采集未启动，请重试并确认系统授权", Toast.LENGTH_LONG).show();
        }
    };
    private android.window.OnBackInvokedCallback systemBack;
    private LinearLayout audioSettingsRoot;
    private LinearLayout lanQualityCard;
    private TextView captureStateText;
    private Switch captureSwitch;
    private Switch lanEnableSwitch;
    private Switch bluetoothEnableSwitch;
    private RefreshControl lanRefreshButton;
    private RefreshControl bluetoothRefreshButton;
    private TextView lanSectionStatus;
    private TextView bluetoothSectionStatus;
    private ExpandableTransportBody lanSectionBody;
    private ExpandableTransportBody bluetoothSectionBody;
    private LinearLayout lanDeviceList;
    private LinearLayout bluetoothDeviceList;
    private TextView lanTimerText;
    private TextView bluetoothTimerText;
    private boolean settingsOpen;
    private String settingsCategory;
    private DiscoverySession discoverySession;
    private boolean statusReceiverRegistered;
    private ObjectAnimator lanRefreshAnimator;
    private ObjectAnimator bluetoothRefreshAnimator;
    private boolean capturing;
    private boolean lanConnected;
    private boolean bluetoothConnected;
    private AlertDialog audioSettingsLockedDialog;
    private long lanConnectedAtElapsedRealtime;
    private long bluetoothConnectedAtElapsedRealtime;
    private String lanName = "局域网设备";
    private String lanDeviceId = "";
    private String bluetoothName = "蓝牙设备";
    private String bluetoothDeviceId = "";
    private String bluetoothAddress = "";
    private String latestStatus = "正在初始化音频连接";

    private final Runnable timerTick = new Runnable() {
        @Override public void run() {
            updateConnectionTimers();
            handler.postDelayed(this, 1000);
        }
    };

    private final BroadcastReceiver statusReceiver = new BroadcastReceiver() {
        @Override public void onReceive(Context context, Intent intent) {
            String status = intent.getStringExtra(AudioRouterService.EXTRA_STATUS);
            if (status != null) {
                latestStatus = status;
                if (lanSectionStatus != null && lanEnableSwitch != null
                        && lanEnableSwitch.isChecked() && status.contains("局域网")) {
                    lanSectionStatus.setText(status);
                }
                if (bluetoothSectionStatus != null && bluetoothEnableSwitch != null
                        && bluetoothEnableSwitch.isChecked() && status.contains("蓝牙")) {
                    bluetoothSectionStatus.setText(status);
                }
                if (status.contains("失败") || status.contains("权限不足")) {
                    Toast.makeText(MainActivity.this, status, Toast.LENGTH_LONG).show();
                }
            }
            if (intent.hasExtra(AudioRouterService.EXTRA_LAN_LISTEN_ENABLED)
                    && lanEnableSwitch != null) {
                updatingTransportSwitches = true;
                lanEnableSwitch.setChecked(intent.getBooleanExtra(
                        AudioRouterService.EXTRA_LAN_LISTEN_ENABLED, false));
                updatingTransportSwitches = false;
            }
            if (intent.hasExtra(AudioRouterService.EXTRA_BLUETOOTH_LISTEN_ENABLED)
                    && bluetoothEnableSwitch != null) {
                updatingTransportSwitches = true;
                bluetoothEnableSwitch.setChecked(intent.getBooleanExtra(
                        AudioRouterService.EXTRA_BLUETOOTH_LISTEN_ENABLED, false));
                updatingTransportSwitches = false;
            }
            if (intent.hasExtra(AudioRouterService.EXTRA_CAPTURING)) {
                capturing = intent.getBooleanExtra(AudioRouterService.EXTRA_CAPTURING, false);
                if (capturing || (status != null && (status.contains("失败")
                        || status.contains("录制已停止") || status.contains("授权无效")
                        || status.contains("请稍后重试")))) {
                    capturePending = false;
                    handler.removeCallbacks(captureStartTimeout);
                }
                updateCaptureUi();
            }

            boolean oldLanConnected = lanConnected;
            boolean oldBluetoothConnected = bluetoothConnected;
            String oldLanName = lanName;
            String oldBluetoothName = bluetoothName;
            String oldLanDeviceId = lanDeviceId;
            String oldBluetoothDeviceId = bluetoothDeviceId;
            long oldLanTime = lanConnectedAtElapsedRealtime;
            long oldBluetoothTime = bluetoothConnectedAtElapsedRealtime;
            lanConnected = intent.getBooleanExtra(AudioRouterService.EXTRA_LAN_CONNECTED, false);
            bluetoothConnected = intent.getBooleanExtra(
                    AudioRouterService.EXTRA_BLUETOOTH_CONNECTED, false);
            lanName = intent.getStringExtra(AudioRouterService.EXTRA_LAN_NAME);
            if (lanName == null || lanName.trim().isEmpty()) lanName = "局域网设备";
            lanDeviceId = intent.getStringExtra(AudioRouterService.EXTRA_LAN_DEVICE_ID);
            if (lanDeviceId == null) lanDeviceId = "";
            bluetoothName = intent.getStringExtra(AudioRouterService.EXTRA_BLUETOOTH_NAME);
            if (bluetoothName == null || bluetoothName.trim().isEmpty()) bluetoothName = "蓝牙设备";
            bluetoothDeviceId = intent.getStringExtra(AudioRouterService.EXTRA_BLUETOOTH_DEVICE_ID);
            if (bluetoothDeviceId == null) bluetoothDeviceId = "";
            bluetoothAddress = intent.getStringExtra(AudioRouterService.EXTRA_BLUETOOTH_ADDRESS);
            if (bluetoothAddress == null) bluetoothAddress = "";
            lanConnectedAtElapsedRealtime = intent.getLongExtra(
                    AudioRouterService.EXTRA_LAN_CONNECTED_AT, 0);
            bluetoothConnectedAtElapsedRealtime = intent.getLongExtra(
                    AudioRouterService.EXTRA_BLUETOOTH_CONNECTED_AT, 0);

            if (!pendingLanIdentity.isEmpty() && ((lanConnected
                    && pendingLanIdentity.equalsIgnoreCase(lanDeviceId))
                    || (status != null && status.contains("局域网")
                    && (status.contains("失败") || status.contains("断开"))))) {
                pendingLanIdentity = "";
                handler.removeCallbacks(lanConnectionTimeout);
            }
            if (!pendingBluetoothIdentity.isEmpty() && ((bluetoothConnected
                    && pendingBluetoothIdentity.equalsIgnoreCase(bluetoothAddress))
                    || (status != null && status.contains("蓝牙")
                    && (status.contains("失败") || status.contains("断开"))))) {
                pendingBluetoothIdentity = "";
                handler.removeCallbacks(bluetoothConnectionTimeout);
            }

            if (oldLanConnected != lanConnected || oldBluetoothConnected != bluetoothConnected
                    || !oldLanName.equals(lanName) || !oldBluetoothName.equals(bluetoothName)
                    || !oldLanDeviceId.equals(lanDeviceId)
                    || !oldBluetoothDeviceId.equals(bluetoothDeviceId)
                    || oldLanTime != lanConnectedAtElapsedRealtime
                    || oldBluetoothTime != bluetoothConnectedAtElapsedRealtime) {
                renderConnectedCards();
            }
            if (status != null && (status.contains("失败") || status.contains("断开"))) {
                renderConnectedCards();
            }
            updateAudioSettingsLockAppearance();
            if ("请关闭传输后再调整设置".equals(status)) showAudioSettingsLockedDialog();
            updateConnectionSummary();
        }
    };

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        setVolumeControlStream(android.media.AudioManager.STREAM_MUSIC);
        applyThemePalette();
        applySystemBars();
        pageHost = new FrameLayout(this);
        pageHost.setBackgroundColor(COLOR_BACKGROUND);
        setContentView(pageHost);
        if (savedInstanceState != null) {
            settingsOpen = savedInstanceState.getBoolean("settings_open");
            settingsCategory = savedInstanceState.getString("settings_category");
            homeScrollY = savedInstanceState.getInt("home_scroll_y");
        }
        homePage = createContentView();
        currentPage = settingsOpen ? createSettingsView() : homePage;
        pageHost.addView(currentPage);
        updateSystemBack();
        if (!settingsOpen) homePage.post(() -> homePage.scrollTo(0, homeScrollY));
        updateCaptureUi();
        updateConnectionSummary();
        refreshDevices();
        refreshLanDevices();
    }

    @Override
    protected void onStart() {
        super.onStart();
        IntentFilter filter = new IntentFilter(AudioRouterService.ACTION_STATUS);
        if (Build.VERSION.SDK_INT >= 33) {
            registerReceiver(statusReceiver, filter, Context.RECEIVER_NOT_EXPORTED);
        } else {
            registerReceiver(statusReceiver, filter);
        }
        statusReceiverRegistered = true;
        sendServiceIntent(new Intent(this, AudioRouterService.class)
                .setAction(AudioRouterService.ACTION_LISTEN));
        handler.removeCallbacks(timerTick);
        handler.post(timerTick);
    }

    @Override
    protected void onStop() {
        handler.removeCallbacks(timerTick);
        if (statusReceiverRegistered) {
            unregisterReceiver(statusReceiver);
            statusReceiverRegistered = false;
        }
        super.onStop();
    }

    @Override
    protected void onSaveInstanceState(Bundle state) {
        super.onSaveInstanceState(state);
        state.putBoolean("settings_open", settingsOpen);
        state.putString("settings_category", settingsCategory);
        state.putInt("home_scroll_y", homePage == null ? homeScrollY : homePage.getScrollY());
    }

    @Override
    protected void onDestroy() {
        if (audioSettingsLockedDialog != null) audioSettingsLockedDialog.dismiss();
        if (discoverySession != null) {
            discoverySession.close();
            discoverySession = null;
        }
        setRefreshing(false, true);
        setRefreshing(false, false);
        if (Build.VERSION.SDK_INT >= 33 && systemBack != null) {
            getOnBackInvokedDispatcher().unregisterOnBackInvokedCallback(systemBack);
            systemBack = null;
        }
        if (pageHost != null) {
            for (int index = 0; index < pageHost.getChildCount(); index++) {
                pageHost.getChildAt(index).animate().cancel();
            }
        }
        handler.removeCallbacksAndMessages(null);
        super.onDestroy();
    }

    @Override
    public void onBackPressed() {
        if (settingsOpen) {
            if (settingsCategory != null) {
                showSettingsOverview();
            } else {
                showHome();
            }
            return;
        }
        super.onBackPressed();
    }

    private void applyThemePalette() {
        if (isDarkTheme()) {
            COLOR_BACKGROUND = Color.rgb(14, 23, 21);
            COLOR_SURFACE = Color.rgb(24, 38, 34);
            COLOR_SURFACE_ALT = Color.rgb(32, 51, 46);
            COLOR_LINE = Color.rgb(59, 85, 77);
            COLOR_TEXT = Color.rgb(238, 246, 241);
            COLOR_MUTED = Color.rgb(155, 180, 170);
            COLOR_MINT = Color.rgb(143, 221, 195);
            COLOR_AMBER = Color.rgb(242, 174, 112);
        } else {
            COLOR_BACKGROUND = Color.rgb(246, 248, 246);
            COLOR_SURFACE = Color.rgb(255, 255, 255);
            COLOR_SURFACE_ALT = Color.rgb(231, 240, 235);
            COLOR_LINE = Color.rgb(210, 224, 216);
            COLOR_TEXT = Color.rgb(24, 51, 42);
            COLOR_MUTED = Color.rgb(103, 126, 115);
            COLOR_MINT = Color.rgb(31, 121, 94);
            COLOR_AMBER = Color.rgb(193, 103, 42);
        }
    }

    private void applySystemBars() {
        Window window = getWindow();
        window.setStatusBarColor(COLOR_BACKGROUND);
        window.setNavigationBarColor(COLOR_BACKGROUND);
        if (Build.VERSION.SDK_INT >= 29) {
            window.setStatusBarContrastEnforced(false);
            window.setNavigationBarContrastEnforced(false);
        }
        int flags = 0;
        if (!isDarkTheme()) {
            flags |= View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR;
            if (Build.VERSION.SDK_INT >= 26) flags |= View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR;
        }
        window.getDecorView().setSystemUiVisibility(flags);
    }

    private String themeMode() {
        String mode = settings().getString(PREF_THEME_MODE, THEME_SYSTEM);
        if (!THEME_LIGHT.equals(mode) && !THEME_DARK.equals(mode)) return THEME_SYSTEM;
        return mode;
    }

    private boolean isDarkTheme() {
        String mode = themeMode();
        if (THEME_DARK.equals(mode)) return true;
        if (THEME_LIGHT.equals(mode)) return false;
        int nightMask = getResources().getConfiguration().uiMode
                & android.content.res.Configuration.UI_MODE_NIGHT_MASK;
        return nightMask == android.content.res.Configuration.UI_MODE_NIGHT_YES;
    }

    private void showSettings() {
        homeScrollY = homePage.getScrollY();
        settingsOpen = true;
        settingsCategory = null;
        overviewPage = createSettingsOverviewView();
        navigateTo(overviewPage, true);
    }

    private void showHome() {
        settingsOpen = false;
        settingsCategory = null;
        if (homePage == null) homePage = createContentView();
        navigateTo(homePage, false);
        homePage.post(() -> homePage.scrollTo(0, homeScrollY));
        updateCaptureUi();
        updateConnectionSummary();
        renderConnectedCards();
    }

    private void showSettingsOverview() {
        settingsCategory = null;
        if (overviewPage == null) overviewPage = createSettingsOverviewView();
        navigateTo(overviewPage, false);
    }

    private void navigateTo(ScrollView next, boolean forward) {
        updateSystemBack();
        ScrollView previous = currentPage;
        if (previous == next) return;
        for (int index = pageHost.getChildCount() - 1; index >= 0; index--) {
            View child = pageHost.getChildAt(index);
            child.animate().cancel();
            if (child != previous) pageHost.removeView(child);
        }
        if (next.getParent() != null) ((ViewGroup) next.getParent()).removeView(next);
        currentPage = next;
        pageHost.addView(next, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        next.requestApplyInsets();
        boolean animate = ValueAnimator.areAnimatorsEnabled();
        float offset = dp(18) * (forward ? 1 : -1);
        next.setTranslationX(animate ? offset : 0);
        next.setAlpha(animate ? 0 : 1);
        next.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_AUTO);
        if (previous != null) {
            previous.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS);
            previous.setTranslationX(0);
            previous.setAlpha(1);
            previous.setOnTouchListener((view, event) -> true);
            if (animate) {
                previous.animate().translationX(-offset).alpha(0).setDuration(280)
                        .setInterpolator(new DecelerateInterpolator(2f))
                        .withEndAction(() -> pageHost.removeView(previous)).start();
            } else {
                pageHost.removeView(previous);
            }
        }
        next.setOnTouchListener(null);
        if (animate) next.animate().translationX(0).alpha(1).setDuration(340)
                .setInterpolator(new DecelerateInterpolator(2f)).start();
    }

    private void rebuildCurrentSettings() {
        int scrollY = currentPage.getScrollY();
        currentPage.animate().cancel();
        ScrollView next = createSettingsView();
        next.setFocusableInTouchMode(true);
        next.requestFocus();
        next.getViewTreeObserver().addOnPreDrawListener(new android.view.ViewTreeObserver.OnPreDrawListener() {
            @Override
            public boolean onPreDraw() {
                next.getViewTreeObserver().removeOnPreDrawListener(this);
                next.scrollTo(0, scrollY);
                return true;
            }
        });
        pageHost.removeAllViews();
        currentPage = next;
        pageHost.addView(next, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        next.requestApplyInsets();
    }

    private void refreshHomeDisplay() {
        ScrollView previousHome = homePage;
        int scrollY = previousHome == null ? homeScrollY : previousHome.getScrollY();
        homePage = createContentView();
        homePage.post(() -> homePage.scrollTo(0, scrollY));
        renderConnectedCards();
        if (!settingsOpen && currentPage == previousHome) {
            navigateTo(homePage, false);
        }
    }

    private void updateSystemBack() {
        if (Build.VERSION.SDK_INT < 33) return;
        if (settingsOpen && systemBack == null) {
            systemBack = this::onBackPressed;
            getOnBackInvokedDispatcher().registerOnBackInvokedCallback(
                    android.window.OnBackInvokedDispatcher.PRIORITY_DEFAULT, systemBack);
        } else if (!settingsOpen && systemBack != null) {
            getOnBackInvokedDispatcher().unregisterOnBackInvokedCallback(systemBack);
            systemBack = null;
        }
    }

    private ScrollView createSettingsView() {
        if ("appearance".equals(settingsCategory)) {
            return createAppearanceSettingsView();
        }
        if ("connection".equals(settingsCategory)) {
            return createConnectionSettingsView();
        }
        if ("audio".equals(settingsCategory)) {
            return createAudioQualitySettingsView();
        }
        if ("buffer".equals(settingsCategory)) {
            return createAudioBufferSettingsView();
        }
        if ("about".equals(settingsCategory)) {
            return createAboutSettingsView();
        }
        return createSettingsOverviewView();
    }

    private ScrollView createSettingsOverviewView() {
        LinearLayout root = column();
        ScrollView scroll = createSettingsScrollView(root);
        addSettingsTopBar(root, "设置", this::showHome);

        addSettingsCategory(root, "外观", "界面主题和颜色模式", "appearance");
        addSettingsCategory(root, "个性化", "个性化选项", "connection");
        addSettingsCategory(root, "音频传输", "压缩方式、传输速率和蓝牙协议", "audio");
        addSettingsCategory(root, "音频缓冲", "调整缓冲选项", "buffer");
        addSettingsCategory(root, "关于", "应用信息、数据存储与第三方组件", "about");
        return scroll;
    }

    private ScrollView createAboutSettingsView() {
        LinearLayout root = column();
        ScrollView scroll = createSettingsScrollView(root);
        addSettingsTopBar(root, "关于", this::showSettingsOverview);

        LinearLayout identity = column();
        identity.setGravity(Gravity.CENTER_HORIZONTAL);
        LinearLayout.LayoutParams identityParams = matchWrap();
        identityParams.topMargin = dp(26);
        identityParams.bottomMargin = dp(28);
        root.addView(identity, identityParams);

        ImageView appIcon = new ImageView(this);
        appIcon.setImageResource(R.mipmap.ic_launcher);
        appIcon.setContentDescription("AudioRouter 应用图标");
        appIcon.setScaleType(ImageView.ScaleType.CENTER_CROP);
        LinearLayout.LayoutParams iconParams = new LinearLayout.LayoutParams(dp(96), dp(96));
        iconParams.gravity = Gravity.CENTER_HORIZONTAL;
        identity.addView(appIcon, iconParams);

        TextView name = text("AudioRouter", 22, COLOR_TEXT);
        name.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        name.setGravity(Gravity.CENTER);
        LinearLayout.LayoutParams nameParams = wrapWrap();
        nameParams.topMargin = dp(10);
        identity.addView(name, nameParams);

        String version;
        try {
            version = getPackageManager().getPackageInfo(getPackageName(), 0).versionName;
        } catch (PackageManager.NameNotFoundException error) {
            version = "1.0.0";
        }
        TextView versionText = text("V" + version, 12, COLOR_MUTED);
        versionText.setGravity(Gravity.CENTER);
        identity.addView(versionText, wrapWrap());

        TextView author = text("By CutePillow", 12, COLOR_MUTED);
        author.setGravity(Gravity.CENTER);
        identity.addView(author, wrapWrap());

        LinearLayout metadata = column();
        root.addView(metadata, matchWrap());
        addAboutLine(metadata);
        String appDataDirectory = getApplicationInfo().dataDir;
        addAboutMetadataRow(metadata, "数据存储位置",
                "应用数据目录：" + appDataDirectory + "\n设置文件："
                        + appDataDirectory + "/shared_prefs/" + PREFS_NAME + ".xml", null);
        addAboutMetadataRow(metadata, "项目地址", "github.com/CutePillow/AudioRouter",
                "https://github.com/CutePillow/AudioRouter");

        LinearLayout information = column();
        LinearLayout.LayoutParams informationParams = matchWrap();
        informationParams.topMargin = dp(12);
        root.addView(information, informationParams);

        LinearLayout licenseRow = row();
        licenseRow.setGravity(Gravity.CENTER_VERTICAL);
        licenseRow.setPadding(dp(3), dp(12), dp(3), dp(12));
        TextView licenseTitle = text("项目许可证", 14, COLOR_TEXT);
        licenseTitle.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        licenseRow.addView(licenseTitle, new LinearLayout.LayoutParams(0,
                ViewGroup.LayoutParams.WRAP_CONTENT, 1));
        ImageView licenseChevron = icon(R.drawable.ic_chevron_right, COLOR_TEXT);
        licenseRow.addView(licenseChevron, new LinearLayout.LayoutParams(dp(20), dp(20)));
        information.addView(licenseRow, matchWrap());

        ExpandableTransportBody licenseDetail = new ExpandableTransportBody(this);
        TextView licenseText = text(
                "GNU General Public License v3.0（GPL-3.0-only）。第三方组件遵循各自的许可证。",
                12, COLOR_MUTED);
        licenseText.setLineSpacing(dp(3), 1f);
        licenseText.setPadding(dp(28), dp(3), 0, dp(14));
        licenseText.setHorizontallyScrolling(false);
        licenseDetail.addView(licenseText, matchWrap());
        information.addView(licenseDetail, matchWrap());
        licenseDetail.setExpanded(false);
        licenseRow.setContentDescription("项目许可证，展开或收起许可证说明");
        licenseRow.setOnClickListener(view -> {
            boolean expanded = licenseDetail.getVisibility() != View.VISIBLE;
            licenseDetail.setExpanded(expanded);
            if (ValueAnimator.areAnimatorsEnabled()) {
                licenseChevron.animate().rotation(expanded ? 90f : 0f).setDuration(220).start();
            } else {
                licenseChevron.setRotation(expanded ? 90f : 0f);
            }
        });

        TextView credits = text("第三方组件与致谢\nAndroid：Android SDK、Concentus\n"
                + "Windows：Rust、Tao、Wry / WebView2、Opus\n感谢相关开源项目与贡献者。",
                10, COLOR_MUTED);
        credits.setGravity(Gravity.CENTER);
        credits.setLineSpacing(dp(4), 1f);
        LinearLayout.LayoutParams creditsParams = matchWrap();
        creditsParams.topMargin = dp(54);
        creditsParams.leftMargin = dp(8);
        creditsParams.rightMargin = dp(8);
        root.addView(credits, creditsParams);
        return scroll;
    }

    private void addAboutMetadataRow(LinearLayout parent, String title, String value, String url) {
        LinearLayout row = column();
        row.setPadding(dp(3), dp(10), dp(3), dp(10));
        TextView heading = text(title, 14, COLOR_TEXT);
        heading.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        row.addView(heading, matchWrap());
        TextView detail = text(value, 11, url == null ? COLOR_MUTED : COLOR_MINT);
        detail.setLineSpacing(dp(3), 1f);
        detail.setHorizontallyScrolling(false);
        detail.setTextIsSelectable(url == null);
        if (url != null) {
            detail.setFocusable(true);
            detail.setContentDescription("在默认浏览器中打开项目地址");
            detail.setOnClickListener(view -> openExternalUrl(url));
        }
        LinearLayout.LayoutParams detailParams = matchWrap();
        detailParams.topMargin = dp(4);
        row.addView(detail, detailParams);
        parent.addView(row, matchWrap());
        addAboutLine(parent);
    }

    private void addAboutLine(LinearLayout parent) {
        View line = new View(this);
        line.setBackgroundColor(COLOR_LINE);
        parent.addView(line, new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, dp(1)));
    }

    private void openExternalUrl(String url) {
        Intent intent = new Intent(Intent.ACTION_VIEW, Uri.parse(url));
        try {
            startActivity(intent);
        } catch (ActivityNotFoundException error) {
            Toast.makeText(this, "未找到可打开链接的浏览器", Toast.LENGTH_LONG).show();
        }
    }

    private ScrollView createSettingsScrollView(LinearLayout root) {
        ScrollView scroll = new ScrollView(this);
        scroll.setFillViewport(true);
        scroll.setBackgroundColor(COLOR_BACKGROUND);
        scroll.setClipToPadding(false);
        root.setPadding(dp(18), dp(18), dp(24), dp(36));
        scroll.addView(root, new ScrollView.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        applySafeInsets(scroll, root, dp(18), dp(36));
        return scroll;
    }

    private void addSettingsTopBar(LinearLayout root, String title, Runnable backAction) {
        LinearLayout topBar = row();
        topBar.setGravity(Gravity.CENTER_VERTICAL);
        ImageView back = icon(R.drawable.ic_chevron_left, COLOR_TEXT);
        back.setPadding(dp(11), dp(11), dp(11), dp(11));
        back.setContentDescription("返回");
        back.setFocusable(true);
        back.setBackground(rounded(COLOR_SURFACE_ALT, COLOR_LINE, 14));
        back.setOnClickListener(view -> backAction.run());
        topBar.addView(back, new LinearLayout.LayoutParams(dp(46), dp(46)));

        LinearLayout titleStack = column();
        TextView eyebrow = text("设置".equals(title) ? "Workspace" : "设置",
                10, COLOR_MUTED);
        titleStack.addView(eyebrow, matchWrap());
        TextView titleText = text(title, 25, COLOR_TEXT);
        titleText.setTypeface(Typeface.create("sans-serif-medium", Typeface.NORMAL));
        titleStack.addView(titleText, matchWrap());
        LinearLayout.LayoutParams titleParams = new LinearLayout.LayoutParams(0,
                ViewGroup.LayoutParams.WRAP_CONTENT, 1);
        titleParams.leftMargin = dp(13);
        topBar.addView(titleStack, titleParams);
        root.addView(topBar, matchWrap());
    }

    private void addSettingsCategory(LinearLayout root, String title, String detail,
                                     String category) {
        LinearLayout item = row();
        item.setGravity(Gravity.CENTER_VERTICAL);
        item.setPadding(dp(15), dp(14), dp(13), dp(14));
        item.setBackground(rounded(COLOR_SURFACE, COLOR_LINE, 14));
        LinearLayout labels = column();
        TextView titleText = text(title, 15, COLOR_TEXT);
        titleText.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        labels.addView(titleText, matchWrap());
        TextView detailText = text(detail, 11, COLOR_MUTED);
        LinearLayout.LayoutParams detailParams = matchWrap();
        detailParams.topMargin = dp(3);
        labels.addView(detailText, detailParams);
        LinearLayout.LayoutParams labelsParams = new LinearLayout.LayoutParams(0,
                ViewGroup.LayoutParams.WRAP_CONTENT, 1);
        item.addView(labels, labelsParams);
        ImageView arrow = icon(R.drawable.ic_chevron_right, COLOR_MUTED);
        item.addView(arrow, new LinearLayout.LayoutParams(dp(24), dp(36)));
        LinearLayout.LayoutParams itemParams = matchWrap();
        itemParams.topMargin = dp(10);
        root.addView(item, itemParams);
        item.setContentDescription(title + "，" + detail);
        if ("audio".equals(category) || "buffer".equals(category)) {
            item.setTag("protected_audio_settings");
            item.setAlpha(isAudioSettingsLocked() ? 0.45f : 1f);
        }
        item.setOnClickListener(view -> {
            if (("audio".equals(category) || "buffer".equals(category))
                    && blockAudioSettingsChange()) return;
            settingsCategory = category;
            navigateTo(createSettingsView(), true);
        });
    }

    private boolean isAudioSettingsLocked() {
        return capturing || capturePending || lanConnected || bluetoothConnected
                || !pendingLanIdentity.isEmpty() || !pendingBluetoothIdentity.isEmpty();
    }

    private boolean blockAudioSettingsChange() {
        if (!isAudioSettingsLocked()) return false;
        showAudioSettingsLockedDialog();
        return true;
    }

    private void updateAudioSettingsLockAppearance() {
        boolean locked = isAudioSettingsLocked();
        if (pageHost != null) applyAudioSettingsLockAppearance(pageHost, locked, false);
        if (overviewPage != null && overviewPage.getParent() == null) {
            applyAudioSettingsLockAppearance(overviewPage, locked, false);
        }
    }

    private void applyAudioSettingsLockAppearance(View view, boolean locked, boolean protectedArea) {
        if ("protected_audio_settings".equals(view.getTag())) {
            view.setAlpha(locked ? 0.45f : 1f);
            protectedArea = true;
        }
        if (protectedArea && view instanceof EditText) view.setEnabled(!locked);
        if (view instanceof ViewGroup) {
            ViewGroup group = (ViewGroup) view;
            for (int index = 0; index < group.getChildCount(); index++) {
                applyAudioSettingsLockAppearance(group.getChildAt(index), locked, protectedArea);
            }
        }
    }

    private void showAudioSettingsLockedDialog() {
        if (audioSettingsLockedDialog != null && audioSettingsLockedDialog.isShowing()) return;
        LinearLayout content = column();
        content.setPadding(dp(22), dp(22), dp(22), dp(18));
        content.setBackground(rounded(COLOR_SURFACE, COLOR_LINE, 18));
        TextView title = text("暂不可调整", 17, COLOR_TEXT);
        title.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        content.addView(title, matchWrap());
        TextView message = text("请关闭传输后再调整设置", 14, COLOR_MUTED);
        LinearLayout.LayoutParams messageParams = matchWrap();
        messageParams.topMargin = dp(12);
        messageParams.bottomMargin = dp(20);
        content.addView(message, messageParams);
        Button confirm = button("知道了", false);
        confirm.setTextColor(COLOR_BACKGROUND);
        confirm.setBackground(rounded(COLOR_MINT, COLOR_MINT, 10));
        content.addView(confirm, new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, dp(44)));
        AlertDialog dialog = new AlertDialog.Builder(this,
                isDarkTheme() ? android.R.style.Theme_Material_Dialog_Alert
                        : android.R.style.Theme_Material_Light_Dialog_Alert).create();
        dialog.setView(content, 0, 0, 0, 0);
        confirm.setOnClickListener(view -> dialog.dismiss());
        audioSettingsLockedDialog = dialog;
        dialog.show();
        if (dialog.getWindow() != null) {
            dialog.getWindow().setBackgroundDrawable(new android.graphics.drawable.ColorDrawable(Color.TRANSPARENT));
        }
    }

    private ScrollView createAppearanceSettingsView() {
        LinearLayout root = column();
        ScrollView scroll = createSettingsScrollView(root);

        addSettingsTopBar(root, "外观", this::showSettingsOverview);

        LinearLayout appearanceCard = column();
        appearanceCard.setPadding(dp(16), dp(16), dp(16), dp(16));
        appearanceCard.setBackground(rounded(COLOR_SURFACE, COLOR_LINE, 18));
        LinearLayout.LayoutParams cardParams = matchWrap();
        cardParams.topMargin = dp(18);
        root.addView(appearanceCard, cardParams);

        TextView kicker = text("外观", 11, COLOR_AMBER);
        kicker.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        appearanceCard.addView(kicker, matchWrap());
        TextView heading = text("界面主题", 19, COLOR_TEXT);
        heading.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        LinearLayout.LayoutParams headingParams = matchWrap();
        headingParams.topMargin = dp(4);
        appearanceCard.addView(heading, headingParams);
        addThemeOption(appearanceCard, THEME_LIGHT, "浅色");
        addThemeOption(appearanceCard, THEME_DARK, "深色");
        addThemeOption(appearanceCard, THEME_SYSTEM, "跟随系统");

        return scroll;
    }

    private ScrollView createConnectionSettingsView() {
        LinearLayout root = column();
        ScrollView scroll = createSettingsScrollView(root);
        addSettingsTopBar(root, "个性化", this::showSettingsOverview);
        addPersonalizationSetting(root, "连接前显示音频控制选项",
                "开启后，连接设备前也可以提前调整发送、接收和音量设置。",
                PREF_PRECONNECT_CONTROLS, false);
        addPersonalizationSetting(root, "显示 IP 地址",
                "在局域网设备列表中显示设备的 IP 地址。",
                PREF_SHOW_IP_ADDRESS, false);
        addPersonalizationSetting(root, "应用上次关闭时的连接状态",
                "下次启动时恢复上次关闭前已开启的局域网和蓝牙连接。",
                PREF_RESTORE_CONNECTION_STATE, true);
        addAutoConnectSetting(root);
        return scroll;
    }

    private void addAutoConnectSetting(LinearLayout root) {
        LinearLayout card = column();
        card.setPadding(dp(14), dp(12), dp(12), dp(12));
        card.setBackground(rounded(COLOR_SURFACE, COLOR_LINE, 14));

        LinearLayout header = row();
        header.setGravity(Gravity.CENTER_VERTICAL);
        LinearLayout copy = column();
        TextView title = text("自动尝试连接", 14, COLOR_TEXT);
        title.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        copy.addView(title, matchWrap());
        TextView detail = text("设备连接失败时，自动按设定的次数和间隔重试。", 10, COLOR_MUTED);
        LinearLayout.LayoutParams detailParams = matchWrap();
        detailParams.topMargin = dp(3);
        copy.addView(detail, detailParams);
        LinearLayout.LayoutParams copyParams = new LinearLayout.LayoutParams(0,
                ViewGroup.LayoutParams.WRAP_CONTENT, 1);
        copyParams.rightMargin = dp(8);
        header.addView(copy, copyParams);

        Switch enabled = new Switch(this);
        styleSwitch(enabled);
        enabled.setShowText(false);
        enabled.setContentDescription("自动尝试连接");
        enabled.setChecked(settings().getBoolean(PREF_AUTO_CONNECT, true));
        header.addView(enabled, wrapWrap());
        card.addView(header, matchWrap());

        LinearLayout options = column();
        options.setPadding(dp(10), dp(10), dp(10), dp(8));
        options.setBackground(rounded(COLOR_SURFACE_ALT, COLOR_LINE, 10));
        TextView attempts = text("", 12, COLOR_TEXT);
        TextView interval = text("", 12, COLOR_TEXT);
        Button attemptsButton = button("", false);
        Button intervalButton = button("", false);
        attemptsButton.setTextSize(12);
        intervalButton.setTextSize(12);
        attemptsButton.setTextColor(COLOR_TEXT);
        intervalButton.setTextColor(COLOR_TEXT);
        attemptsButton.setBackground(rounded(COLOR_SURFACE, COLOR_LINE, 8));
        intervalButton.setBackground(rounded(COLOR_SURFACE, COLOR_LINE, 8));
        LinearLayout attemptsRow = row();
        attemptsRow.setGravity(Gravity.CENTER_VERTICAL);
        LinearLayout.LayoutParams attemptsLabelParams = new LinearLayout.LayoutParams(0,
                ViewGroup.LayoutParams.WRAP_CONTENT, 1);
        attemptsRow.addView(attempts, attemptsLabelParams);
        attemptsRow.addView(attemptsButton, new LinearLayout.LayoutParams(dp(148), dp(40)));
        options.addView(attemptsRow, matchWrap());
        LinearLayout.LayoutParams intervalRowParams = matchWrap();
        intervalRowParams.topMargin = dp(7);
        LinearLayout intervalRow = row();
        intervalRow.setGravity(Gravity.CENTER_VERTICAL);
        LinearLayout.LayoutParams intervalLabelParams = new LinearLayout.LayoutParams(0,
                ViewGroup.LayoutParams.WRAP_CONTENT, 1);
        intervalRow.addView(interval, intervalLabelParams);
        intervalRow.addView(intervalButton, new LinearLayout.LayoutParams(dp(148), dp(40)));
        options.addView(intervalRow, intervalRowParams);
        card.addView(options, matchWrap());

        Runnable sync = () -> {
            boolean checked = settings().getBoolean(PREF_AUTO_CONNECT, true);
            int attemptValue = settings().getInt(PREF_AUTO_CONNECT_ATTEMPTS, 5);
            int intervalValue = settings().getInt(PREF_AUTO_CONNECT_INTERVAL_SECONDS, 5);
            attempts.setText("自动连接尝试次数");
            interval.setText("自动连接尝试间隔");
            attemptsButton.setText(attemptValue == 0 ? "持续尝试" : attemptValue + " 次");
            intervalButton.setText(intervalValue + " 秒");
            options.setVisibility(checked ? View.VISIBLE : View.GONE);
        };
        sync.run();
        enabled.setOnCheckedChangeListener((button, checked) -> {
            settings().edit().putBoolean(PREF_AUTO_CONNECT, checked).apply();
            if (checked) {
                options.setAlpha(0f);
                options.setVisibility(View.VISIBLE);
                options.animate().alpha(1f).setDuration(180).start();
            } else {
                options.animate().alpha(0f).setDuration(140)
                        .withEndAction(() -> options.setVisibility(View.GONE)).start();
            }
        });
        attemptsButton.setOnClickListener(view -> {
            int[] values = {1, 2, 3, 4, 5, 0};
            String[] labels = {"1 次", "2 次", "3 次", "4 次", "5 次", "持续尝试"};
            int selected = settings().getInt(PREF_AUTO_CONNECT_ATTEMPTS, 5);
            showSettingsChoicePopup(attemptsButton, labels, indexOf(values, selected), index -> {
                settings().edit().putInt(PREF_AUTO_CONNECT_ATTEMPTS, values[index]).apply();
                sync.run();
            });
        });
        intervalButton.setOnClickListener(view -> {
            int[] values = {1, 2, 3, 5, 10, 30};
            String[] labels = {"1 秒", "2 秒", "3 秒", "5 秒", "10 秒", "30 秒"};
            int selected = settings().getInt(PREF_AUTO_CONNECT_INTERVAL_SECONDS, 5);
            showSettingsChoicePopup(intervalButton, labels, indexOf(values, selected), index -> {
                settings().edit().putInt(PREF_AUTO_CONNECT_INTERVAL_SECONDS, values[index]).apply();
                sync.run();
            });
        });

        LinearLayout.LayoutParams cardParams = matchWrap();
        cardParams.topMargin = dp(10);
        root.addView(card, cardParams);
    }

    private int indexOf(int[] values, int selected) {
        for (int index = 0; index < values.length; index++) {
            if (values[index] == selected) return index;
        }
        return 0;
    }

    private void showSettingsChoicePopup(Button anchor, String[] labels, int selectedIndex,
                                         IntConsumer onSelected) {
        LinearLayout menu = column();
        menu.setPadding(dp(5), dp(5), dp(5), dp(5));
        menu.setBackground(rounded(COLOR_SURFACE, COLOR_LINE, 9));
        final PopupWindow[] popupHolder = new PopupWindow[1];
        for (int index = 0; index < labels.length; index++) {
            LinearLayout option = row();
            option.setGravity(Gravity.CENTER_VERTICAL);
            option.setPadding(dp(9), 0, dp(9), 0);
            boolean selected = index == selectedIndex;
            option.setBackground(rounded(selected ? COLOR_SURFACE_ALT : Color.TRANSPARENT,
                    selected ? COLOR_MINT : Color.TRANSPARENT, 6));
            TextView label = text(labels[index], 12, selected ? COLOR_MINT : COLOR_TEXT);
            label.setGravity(Gravity.CENTER_VERTICAL);
            option.addView(label, new LinearLayout.LayoutParams(0, dp(34), 1));
            TextView check = text(selected ? "✓" : "", 13, COLOR_MINT);
            check.setGravity(Gravity.CENTER);
            option.addView(check, new LinearLayout.LayoutParams(dp(18), dp(34)));
            option.setContentDescription(labels[index]);
            final int choice = index;
            option.setOnClickListener(view -> {
                onSelected.accept(choice);
                if (popupHolder[0] != null) popupHolder[0].dismiss();
            });
            menu.addView(option, new LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT, dp(34)));
        }
        int width = Math.max(anchor.getWidth(), dp(148));
        PopupWindow popup = new PopupWindow(menu, width, ViewGroup.LayoutParams.WRAP_CONTENT, true);
        popupHolder[0] = popup;
        popup.setBackgroundDrawable(new ColorDrawable(Color.TRANSPARENT));
        popup.setOutsideTouchable(true);
        popup.setElevation(dp(8));
        popup.setAnimationStyle(android.R.style.Animation_Dialog);
        popup.showAsDropDown(anchor, 0, dp(7));
    }

    private void addPersonalizationSetting(LinearLayout root, String title, String detail,
                                          String preference, boolean defaultValue) {
        LinearLayout card = row();
        card.setGravity(Gravity.CENTER_VERTICAL);
        card.setPadding(dp(14), dp(12), dp(12), dp(12));
        card.setBackground(rounded(COLOR_SURFACE, COLOR_LINE, 14));

        LinearLayout copy = column();
        TextView titleText = text(title, 14, COLOR_TEXT);
        titleText.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        copy.addView(titleText, matchWrap());
        TextView detailText = text(detail, 10, COLOR_MUTED);
        LinearLayout.LayoutParams detailParams = matchWrap();
        detailParams.topMargin = dp(3);
        copy.addView(detailText, detailParams);
        LinearLayout.LayoutParams copyParams = new LinearLayout.LayoutParams(0,
                ViewGroup.LayoutParams.WRAP_CONTENT, 1);
        copyParams.rightMargin = dp(8);
        card.addView(copy, copyParams);

        Switch toggle = new Switch(this);
        styleSwitch(toggle);
        toggle.setShowText(false);
        toggle.setContentDescription(title);
        toggle.setChecked(settings().getBoolean(preference, defaultValue));
        toggle.setOnCheckedChangeListener((button, checked) -> {
            settings().edit().putBoolean(preference, checked).apply();
            refreshHomeDisplay();
        });
        card.addView(toggle, wrapWrap());

        LinearLayout.LayoutParams cardParams = matchWrap();
        cardParams.topMargin = dp(10);
        root.addView(card, cardParams);
    }

    private ScrollView createAudioQualitySettingsView() {
        LinearLayout root = column();
        ScrollView scroll = createSettingsScrollView(root);
        addSettingsTopBar(root, "音频传输", this::showSettingsOverview);
        audioSettingsRoot = root;
        lanQualityCard = createLanQualityCard();
        lanQualityCard.setTag("protected_audio_settings");
        LinearLayout.LayoutParams lanParams = matchWrap();
        lanParams.topMargin = dp(18);
        root.addView(lanQualityCard, lanParams);

        LinearLayout bluetoothCard = qualityCard("蓝牙连接", "选择 Bluetooth Classic 音频传输协议。");
        bluetoothCard.setTag("protected_audio_settings");
        addQualityOption(bluetoothCard, "自动", "双方支持时优先使用 Opus",
                AudioQuality.BLUETOOTH_AUTO.equals(bluetoothCodecMode()),
                () -> saveBluetoothAudioQuality(AudioQuality.BLUETOOTH_AUTO));
        addQualityOption(bluetoothCard, "Opus", "压缩传输，降低带宽占用",
                AudioQuality.BLUETOOTH_OPUS.equals(bluetoothCodecMode()),
                () -> saveBluetoothAudioQuality(AudioQuality.BLUETOOTH_OPUS));
        addQualityOption(bluetoothCard, "PCM", "未经压缩，兼容性优先",
                AudioQuality.BLUETOOTH_PCM.equals(bluetoothCodecMode()),
                () -> saveBluetoothAudioQuality(AudioQuality.BLUETOOTH_PCM));
        LinearLayout.LayoutParams bluetoothParams = matchWrap();
        bluetoothParams.topMargin = dp(12);
        root.addView(bluetoothCard, bluetoothParams);
        applyAudioSettingsLockAppearance(scroll, isAudioSettingsLocked(), false);
        return scroll;
    }

    private ScrollView createAudioBufferSettingsView() {
        LinearLayout root = column();
        ScrollView scroll = createSettingsScrollView(root);
        addSettingsTopBar(root, "音频缓冲", this::showSettingsOverview);

        LinearLayout bufferCard = qualityCard("音频缓冲", "缓冲数值越高，传输越稳定，延迟越高");
        bufferCard.setTag("protected_audio_settings");
        int selectedBuffer = AudioRouterService.savedAudioBufferMs(settings());
        boolean customSelected = settings().getBoolean(PREF_CUSTOM_AUDIO_BUFFER_SELECTED,
                !isPresetAudioBuffer(selectedBuffer));
        int[] bufferOptions = new int[]{20, 60, 120, 240};
        for (int bufferMs : bufferOptions) {
            addQualityOption(bufferCard, bufferMs + " ms",
                    bufferMs == 20 ? "低延迟" : bufferMs == 60 ? "平衡"
                            : bufferMs == 120 ? "稳定" : "高稳定",
                    !customSelected && selectedBuffer == bufferMs,
                    () -> saveAudioBuffer(bufferMs));
        }

        LinearLayout customBufferCard = column();
        customBufferCard.setPadding(dp(10), dp(9), dp(10), dp(10));
        customBufferCard.setBackground(rounded(customSelected ? COLOR_SURFACE_ALT : COLOR_SURFACE,
                customSelected ? COLOR_MINT : COLOR_LINE, 12));
        LinearLayout customBufferHeader = row();
        customBufferHeader.setGravity(Gravity.CENTER_VERTICAL);
        LinearLayout customBufferLabels = column();
        TextView customBufferTitle = text("自定义缓冲", 13, COLOR_TEXT);
        customBufferTitle.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        customBufferLabels.addView(customBufferTitle, matchWrap());
        TextView customBufferDetail = text("输入 20–240 ms 的正整数", 10, COLOR_MUTED);
        LinearLayout.LayoutParams customBufferDetailParams = matchWrap();
        customBufferDetailParams.topMargin = dp(2);
        customBufferLabels.addView(customBufferDetail, customBufferDetailParams);
        customBufferHeader.addView(customBufferLabels, new LinearLayout.LayoutParams(0,
                ViewGroup.LayoutParams.WRAP_CONTENT, 1));
        TextView selectedMark = text("已选", 10, COLOR_MINT);
        selectedMark.setVisibility(customSelected ? View.VISIBLE : View.INVISIBLE);
        customBufferHeader.addView(selectedMark, wrapWrap());
        customBufferCard.addView(customBufferHeader, matchWrap());

        EditText customBuffer = new EditText(this);
        customBuffer.setSingleLine(true);
        customBuffer.setInputType(android.text.InputType.TYPE_CLASS_NUMBER);
        customBuffer.setHint("20–240");
        customBuffer.setTextColor(COLOR_TEXT);
        customBuffer.setHintTextColor(COLOR_MUTED);
        customBuffer.setTextSize(13);
        customBuffer.setContentDescription("自定义音频缓冲时间，20 到 240 毫秒");
        customBuffer.setText(settings().getString(PREF_CUSTOM_AUDIO_BUFFER_MS,
                customSelected ? String.valueOf(selectedBuffer) : "20"));
        customBuffer.setImeOptions(android.view.inputmethod.EditorInfo.IME_ACTION_DONE);
        LinearLayout customInputRow = row();
        customInputRow.setGravity(Gravity.CENTER_VERTICAL);
        LinearLayout.LayoutParams customInputRowParams = matchWrap();
        customInputRowParams.topMargin = dp(5);
        customBufferCard.addView(customInputRow, customInputRowParams);
        customInputRow.addView(customBuffer, new LinearLayout.LayoutParams(dp(92), dp(44)));
        TextView customBufferUnit = text("毫秒", 10, COLOR_MUTED);
        LinearLayout.LayoutParams customUnitParams = wrapWrap();
        customUnitParams.leftMargin = dp(8);
        customInputRow.addView(customBufferUnit, customUnitParams);
        customBufferCard.setFocusable(true);
        customBufferCard.setSelected(customSelected);
        customBufferCard.setContentDescription("自定义缓冲，输入 20 到 240 毫秒的正整数"
                + (customSelected ? "，已选择" : ""));
        customBufferCard.setOnClickListener(view -> saveCustomAudioBuffer(customBuffer, true));
        LinearLayout.LayoutParams customOptionParams = matchWrap();
        customOptionParams.topMargin = dp(6);
        bufferCard.addView(customBufferCard, customOptionParams);
        customBuffer.setOnEditorActionListener((view, actionId, event) -> {
            if (blockAudioSettingsChange()) return true;
            if (customSelected) {
                saveCustomAudioBuffer(customBuffer, false);
            } else {
                String draft = customBuffer.getText().toString().trim();
                try {
                    int milliseconds = Integer.parseInt(draft);
                    if (milliseconds >= 20 && milliseconds <= 240) {
                        customBuffer.setError(null);
                        settings().edit().putString(PREF_CUSTOM_AUDIO_BUFFER_MS, draft).apply();
                    } else {
                        customBuffer.setError("请输入 20 到 240 之间的正整数");
                    }
                } catch (NumberFormatException error) {
                    customBuffer.setError("请输入有效的整数");
                }
            }
            return true;
        });
        customBuffer.addTextChangedListener(new TextWatcher() {
            @Override public void beforeTextChanged(CharSequence value, int start, int count, int after) { }
            @Override public void onTextChanged(CharSequence value, int start, int before, int count) {
                if (customBufferSave != null) handler.removeCallbacks(customBufferSave);
                customBufferSave = null;
                if (isAudioSettingsLocked()) return;
                try {
                    int milliseconds = Integer.parseInt(value.toString().trim());
                    if (milliseconds >= 20 && milliseconds <= 240) {
                        customBufferSave = () -> {
                            if (isAudioSettingsLocked()) return;
                            settings().edit().putString(PREF_CUSTOM_AUDIO_BUFFER_MS,
                                    String.valueOf(milliseconds)).apply();
                            if (customSelected) saveCustomAudioBuffer(customBuffer, false);
                        };
                        handler.postDelayed(customBufferSave, 500);
                    }
                } catch (NumberFormatException ignored) {
                }
            }
            @Override public void afterTextChanged(Editable value) { }
        });
        LinearLayout.LayoutParams bufferParams = matchWrap();
        bufferParams.topMargin = dp(18);
        root.addView(bufferCard, bufferParams);
        applyAudioSettingsLockAppearance(scroll, isAudioSettingsLocked(), false);
        return scroll;
    }

    private void saveAudioBuffer(int bufferMs) {
        saveAudioBuffer(bufferMs, true);
    }

    private void saveAudioBuffer(int bufferMs, boolean rebuild) {
        if (blockAudioSettingsChange()) return;
        if (rebuild && customBufferSave != null) {
            handler.removeCallbacks(customBufferSave);
            customBufferSave = null;
        }
        int value = Math.max(20, Math.min(240, bufferMs));
        settings().edit().putInt(AudioRouterService.PREF_AUDIO_BUFFER_MS, value)
                .putBoolean(PREF_CUSTOM_AUDIO_BUFFER_SELECTED, false).apply();
        sendServiceIntent(new Intent(this, AudioRouterService.class)
                .setAction(AudioRouterService.ACTION_SET_AUDIO_BUFFER)
                .putExtra(AudioRouterService.EXTRA_BUFFER_MS, value));
        if (rebuild) rebuildCurrentSettings();
    }

    private boolean isPresetAudioBuffer(int milliseconds) {
        return milliseconds == 20 || milliseconds == 60 || milliseconds == 120 || milliseconds == 240;
    }

    private void saveCustomAudioBuffer(EditText input, boolean rebuild) {
        if (blockAudioSettingsChange()) return;
        String value = input.getText().toString().trim();
        if (value.isEmpty()) {
            input.setError("请输入 20 到 240 之间的正整数");
            return;
        }
        try {
            int bufferMs = Integer.parseInt(value);
            if (bufferMs >= 20 && bufferMs <= 240) {
                if (customBufferSave != null) {
                    handler.removeCallbacks(customBufferSave);
                    customBufferSave = null;
                }
                input.setError(null);
                settings().edit().putString(PREF_CUSTOM_AUDIO_BUFFER_MS, value).apply();
                saveAudioBuffer(bufferMs, false);
                settings().edit().putBoolean(PREF_CUSTOM_AUDIO_BUFFER_SELECTED, true).apply();
                if (rebuild) rebuildCurrentSettings();
            } else {
                input.setError("请输入 20 到 240 之间的正整数");
            }
        } catch (NumberFormatException error) {
            input.setError("请输入有效的整数");
        }
    }

    private LinearLayout createLanQualityCard() {
        LinearLayout lanCard = qualityCard("局域网连接", "支持 Opus 压缩和未经压缩的 PCM 16-bit。");
        addQualityOption(lanCard, "经过压缩", "使用 Opus 降低带宽占用",
                AudioQuality.LAN_COMPRESSED.equals(lanCodecMode()),
                () -> saveLanAudioQuality(AudioQuality.LAN_COMPRESSED, lanOpusBitrateKbps(), lanPcmChannels()));
        addQualityOption(lanCard, "未经压缩", "保持 PCM 16-bit 音频，兼容性优先",
                AudioQuality.LAN_UNCOMPRESSED.equals(lanCodecMode()),
                () -> saveLanAudioQuality(AudioQuality.LAN_UNCOMPRESSED, lanOpusBitrateKbps(), lanPcmChannels()));
        if (AudioQuality.LAN_UNCOMPRESSED.equals(lanCodecMode())) {
            addQualityLabel(lanCard, "未经压缩的传输速率");
            addQualityOption(lanCard, "跟随源设备", "安卓和 Windows 默认双声道 1536 kb/s",
                    lanPcmChannels() == AudioQuality.DEFAULT_PCM_CHANNELS,
                    () -> saveLanAudioQuality(AudioQuality.LAN_UNCOMPRESSED, lanOpusBitrateKbps(), 0));
            addQualityOption(lanCard, "768 kb/s", "48 kHz / 16-bit / 单声道", lanPcmChannels() == 1,
                    () -> saveLanAudioQuality(AudioQuality.LAN_UNCOMPRESSED, lanOpusBitrateKbps(), 1));
            addQualityOption(lanCard, "1536 kb/s", "48 kHz / 16-bit / 双声道", lanPcmChannels() == 2,
                    () -> saveLanAudioQuality(AudioQuality.LAN_UNCOMPRESSED, lanOpusBitrateKbps(), 2));
        } else {
            addQualityLabel(lanCard, "压缩后的传输速率");
            TextView recommendation = text("推荐 96–128 kb/s；256 kb/s 以上适合更看重音质且带宽充足的场景。", 11, COLOR_MUTED);
            LinearLayout.LayoutParams recommendationParams = matchWrap();
            recommendationParams.bottomMargin = dp(8);
            lanCard.addView(recommendation, recommendationParams);
            int[] rates = new int[]{32, 64, 96, 128, 192, 256, 450};
            String[] details = new String[]{"语音、极低带宽", "低带宽、普通音频", "实时传输、均衡",
                    "音乐、推荐", "高质量音乐", "高保真、较高带宽", "极高质量、带宽占用高"};
            LinearLayout gridRow = null;
            for (int index = 0; index < rates.length; index++) {
                int rate = rates[index];
                if (index % 2 == 0) {
                    gridRow = row();
                    lanCard.addView(gridRow, matchWrap());
                }
                LinearLayout cell = column();
                LinearLayout.LayoutParams cellParams = new LinearLayout.LayoutParams(0,
                        ViewGroup.LayoutParams.WRAP_CONTENT, 1);
                if (index % 2 == 0) cellParams.rightMargin = dp(4);
                else cellParams.leftMargin = dp(4);
                gridRow.addView(cell, cellParams);
                addQualityOption(cell, rate + " kb/s", details[index], lanOpusBitrateKbps() == rate,
                        () -> saveLanAudioQuality(AudioQuality.LAN_COMPRESSED, rate, lanPcmChannels()));
                if (index == rates.length - 1) gridRow.addView(new View(this),
                        new LinearLayout.LayoutParams(0, 1, 1));
            }
        }
        return lanCard;
    }

    private LinearLayout qualityCard(String title, String detail) {
        LinearLayout card = column();
        card.setPadding(dp(15), dp(15), dp(15), dp(12));
        card.setBackground(rounded(COLOR_SURFACE, COLOR_LINE, 16));
        TextView titleText = text(title, 17, COLOR_TEXT);
        titleText.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        card.addView(titleText, matchWrap());
        TextView detailText = text(detail, 11, COLOR_MUTED);
        LinearLayout.LayoutParams detailParams = matchWrap();
        detailParams.topMargin = dp(4);
        detailParams.bottomMargin = dp(11);
        card.addView(detailText, detailParams);
        return card;
    }

    private void addQualityLabel(LinearLayout parent, String label) {
        TextView labelText = text(label, 11, COLOR_AMBER);
        labelText.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        LinearLayout.LayoutParams params = matchWrap();
        params.topMargin = dp(8);
        params.bottomMargin = dp(6);
        parent.addView(labelText, params);
    }

    private void addQualityOption(LinearLayout parent, String title, String detail,
                                  boolean selected, Runnable action) {
        LinearLayout option = row();
        option.setGravity(Gravity.CENTER_VERTICAL);
        option.setPadding(dp(12), dp(10), dp(10), dp(10));
        option.setBackground(rounded(selected ? COLOR_SURFACE_ALT : COLOR_SURFACE,
                selected ? COLOR_MINT : COLOR_LINE, 12));
        LinearLayout labels = column();
        TextView titleText = text(title, 13, COLOR_TEXT);
        titleText.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        labels.addView(titleText, matchWrap());
        if (detail != null && !detail.isEmpty()) {
            TextView detailText = text(detail, 10, COLOR_MUTED);
            LinearLayout.LayoutParams detailParams = matchWrap();
            detailParams.topMargin = dp(2);
            labels.addView(detailText, detailParams);
        }
        LinearLayout.LayoutParams labelsParams = new LinearLayout.LayoutParams(0,
                ViewGroup.LayoutParams.WRAP_CONTENT, 1);
        option.addView(labels, labelsParams);
        ImageView marker = icon(R.drawable.ic_check, COLOR_MINT);
        marker.setVisibility(selected ? View.VISIBLE : View.INVISIBLE);
        LinearLayout.LayoutParams markerParams = new LinearLayout.LayoutParams(dp(18), dp(22));
        markerParams.leftMargin = dp(4);
        option.addView(marker, markerParams);
        LinearLayout.LayoutParams optionParams = matchWrap();
        optionParams.bottomMargin = dp(6);
        parent.addView(option, optionParams);
        option.setFocusable(true);
        option.setSelected(selected);
        option.setContentDescription(title + "，" + detail + (selected ? "，已选择" : ""));
        option.setOnClickListener(view -> action.run());
    }

    private String lanCodecMode() {
        return AudioQuality.LAN_UNCOMPRESSED.equals(settings().getString(
                AudioRouterService.PREF_LAN_CODEC_MODE, AudioQuality.LAN_UNCOMPRESSED))
                ? AudioQuality.LAN_UNCOMPRESSED : AudioQuality.LAN_COMPRESSED;
    }

    private int lanOpusBitrateKbps() {
        int rate = settings().getInt(AudioRouterService.PREF_LAN_OPUS_BITRATE_KBPS,
                AudioQuality.DEFAULT_LAN_OPUS_BITRATE_KBPS);
        return Math.max(6, Math.min(450, rate));
    }

    private int lanPcmChannels() {
        int channels = settings().getInt(AudioRouterService.PREF_LAN_PCM_CHANNELS,
                AudioQuality.DEFAULT_PCM_CHANNELS);
        return channels == 1 || channels == 2 ? channels : AudioQuality.DEFAULT_PCM_CHANNELS;
    }

    private String bluetoothCodecMode() {
        String mode = settings().getString(AudioRouterService.PREF_BLUETOOTH_CODEC_MODE,
                AudioQuality.BLUETOOTH_AUTO);
        if (AudioQuality.BLUETOOTH_OPUS.equals(mode) || AudioQuality.BLUETOOTH_PCM.equals(mode)) {
            return mode;
        }
        return AudioQuality.BLUETOOTH_AUTO;
    }

    private void saveLanAudioQuality(String mode, int bitrateKbps, int pcmChannels) {
        if (blockAudioSettingsChange()) return;
        settings().edit()
                .putString(AudioRouterService.PREF_LAN_CODEC_MODE, mode)
                .putInt(AudioRouterService.PREF_LAN_OPUS_BITRATE_KBPS, bitrateKbps)
                .putInt(AudioRouterService.PREF_LAN_PCM_CHANNELS, pcmChannels)
                .apply();
        sendServiceIntent(new Intent(this, AudioRouterService.class)
                .setAction(AudioRouterService.ACTION_SET_LAN_AUDIO_QUALITY)
                .putExtra(AudioRouterService.EXTRA_CODEC_MODE, mode)
                .putExtra(AudioRouterService.EXTRA_OPUS_BITRATE_KBPS, bitrateKbps)
                .putExtra(AudioRouterService.EXTRA_PCM_CHANNELS, pcmChannels));
        if (ValueAnimator.areAnimatorsEnabled()) {
            android.transition.AutoTransition transition = new android.transition.AutoTransition();
            transition.setDuration(220);
            transition.setInterpolator(new DecelerateInterpolator(2f));
            android.transition.TransitionManager.beginDelayedTransition(audioSettingsRoot, transition);
        }
        int cardIndex = audioSettingsRoot.indexOfChild(lanQualityCard);
        ViewGroup.LayoutParams params = lanQualityCard.getLayoutParams();
        audioSettingsRoot.removeView(lanQualityCard);
        lanQualityCard = createLanQualityCard();
        lanQualityCard.setTag("protected_audio_settings");
        audioSettingsRoot.addView(lanQualityCard, cardIndex, params);
    }

    private void saveBluetoothAudioQuality(String mode) {
        if (blockAudioSettingsChange()) return;
        settings().edit().putString(AudioRouterService.PREF_BLUETOOTH_CODEC_MODE, mode).apply();
        sendServiceIntent(new Intent(this, AudioRouterService.class)
                .setAction(AudioRouterService.ACTION_SET_BLUETOOTH_AUDIO_QUALITY)
                .putExtra(AudioRouterService.EXTRA_CODEC_MODE, mode));
        rebuildCurrentSettings();
    }

    private void applySafeInsets(View content, View root, int baseTop, int baseBottom) {
        View.OnApplyWindowInsetsListener insetListener = (view, insets) -> {
            int top;
            int bottom;
            int left;
            int right;
            if (Build.VERSION.SDK_INT >= 30) {
                int types = WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout();
                android.graphics.Insets safe = insets.getInsets(types);
                top = safe.top;
                bottom = safe.bottom;
                left = safe.left;
                right = safe.right;
            } else {
                top = insets.getSystemWindowInsetTop();
                bottom = insets.getSystemWindowInsetBottom();
                left = insets.getSystemWindowInsetLeft();
                right = insets.getSystemWindowInsetRight();
            }
            root.setPadding(dp(18) + left, baseTop + top, dp(24) + right,
                    baseBottom + bottom);
            return insets;
        };
        content.setOnApplyWindowInsetsListener(insetListener);
        WindowInsets currentInsets = getWindow().getDecorView().getRootWindowInsets();
        if (currentInsets != null) insetListener.onApplyWindowInsets(content, currentInsets);
        content.requestApplyInsets();
    }

    private RefreshControl createRefreshButton(String description) {
        RefreshControl button = new RefreshControl(this);
        button.setContentDescription(description);
        button.setFocusable(true);
        button.setClickable(true);
        button.setBackground(rounded(COLOR_SURFACE_ALT, Color.TRANSPARENT, 9));
        button.setVisibility(View.GONE);
        return button;
    }

    private void updateTransportVisibility() {
        if (lanSectionBody != null && lanEnableSwitch != null) {
            boolean enabled = lanEnableSwitch.isChecked();
            lanSectionBody.setExpanded(enabled);
            lanRefreshButton.setVisibility(enabled ? View.VISIBLE : View.GONE);
            if (!enabled) {
                setRefreshing(false, true);
                if (discoverySession != null) {
                    discoverySession.close();
                    discoverySession = null;
                }
            }
        }
        if (bluetoothSectionBody != null && bluetoothEnableSwitch != null) {
            boolean enabled = bluetoothEnableSwitch.isChecked();
            bluetoothSectionBody.setExpanded(enabled);
            bluetoothRefreshButton.setVisibility(enabled ? View.VISIBLE : View.GONE);
            if (!enabled) setRefreshing(false, false);
        }
        updateConnectionSummary();
    }

    private void setRefreshing(boolean refreshing, boolean lan) {
        RefreshControl button = lan ? lanRefreshButton : bluetoothRefreshButton;
        if (button == null) return;
        ObjectAnimator animator = lan ? lanRefreshAnimator : bluetoothRefreshAnimator;
        if (refreshing) {
            button.setEnabled(false);
            button.setContentDescription(lan ? "正在刷新局域网设备" : "正在刷新已配对设备");
            if (!ValueAnimator.areAnimatorsEnabled()) return;
            if (animator == null) {
                animator = ObjectAnimator.ofFloat(button.glyph, View.ROTATION, 0f, -360f);
                animator.setDuration(850);
                animator.setInterpolator(new LinearInterpolator());
                animator.setRepeatCount(ValueAnimator.INFINITE);
                if (lan) lanRefreshAnimator = animator;
                else bluetoothRefreshAnimator = animator;
            }
            if (!animator.isStarted()) animator.start();
        } else {
            button.setEnabled(true);
            button.setContentDescription(lan ? "刷新局域网设备" : "刷新已配对设备");
            if (animator != null) animator.cancel();
            button.glyph.setRotation(0f);
        }
    }

    private void addThemeOption(LinearLayout parent, String mode, String title) {
        boolean selected = mode.equals(themeMode());
        LinearLayout option = row();
        option.setGravity(Gravity.CENTER_VERTICAL);
        option.setPadding(dp(13), dp(12), dp(13), dp(12));
        option.setBackground(rounded(selected ? COLOR_SURFACE_ALT : COLOR_SURFACE,
                selected ? COLOR_MINT : COLOR_LINE, 14));
        option.setContentDescription(title);

        TextView marker = text(selected ? "●" : "○", 20, selected ? COLOR_MINT : COLOR_MUTED);
        marker.setGravity(Gravity.CENTER);
        option.addView(marker, new LinearLayout.LayoutParams(dp(28), dp(28)));

        LinearLayout labels = column();
        TextView titleText = text(title, 14, COLOR_TEXT);
        titleText.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        labels.addView(titleText, matchWrap());
        LinearLayout.LayoutParams labelsParams = new LinearLayout.LayoutParams(0,
                ViewGroup.LayoutParams.WRAP_CONTENT, 1);
        labelsParams.leftMargin = dp(8);
        option.addView(labels, labelsParams);

        LinearLayout.LayoutParams optionParams = matchWrap();
        optionParams.bottomMargin = dp(8);
        parent.addView(option, optionParams);
        option.setOnClickListener(view -> {
            settings().edit().putString(PREF_THEME_MODE, mode).apply();
            setRefreshing(false, true);
            setRefreshing(false, false);
            lanRefreshAnimator = null;
            bluetoothRefreshAnimator = null;
            homeScrollY = homePage.getScrollY();
            applyThemePalette();
            applySystemBars();
            pageHost.setBackgroundColor(COLOR_BACKGROUND);
            homePage = createContentView();
            homePage.post(() -> homePage.scrollTo(0, homeScrollY));
            renderConnectedCards();
            overviewPage = null;
            rebuildCurrentSettings();
        });
    }

    private ScrollView createContentView() {
        ScrollView scroll = new ScrollView(this);
        scroll.setFillViewport(true);
        scroll.setBackgroundColor(COLOR_BACKGROUND);
        scroll.setClipToPadding(false);

        LinearLayout root = column();
        root.setPadding(dp(18), dp(8), dp(24), dp(24));
        scroll.addView(root, new ScrollView.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        applySafeInsets(scroll, root, dp(8), dp(24));

        addBrandHeader(root);
        addSpace(root, 14);
        addCapturePanel(root);
        addSpace(root, 12);
        addLanSection(root);
        addSpace(root, 10);
        addBluetoothSection(root);
        addSpace(root, 18);

        return scroll;
    }

    private void addBrandHeader(LinearLayout root) {
        LinearLayout header = row();
        header.setGravity(Gravity.CENTER_VERTICAL);
        TextView title = text("AudioRouter", 20, COLOR_TEXT);
        title.setTypeface(Typeface.create("sans-serif-medium", Typeface.NORMAL));
        header.addView(title, new LinearLayout.LayoutParams(0, dp(46), 1));
        title.setGravity(Gravity.CENTER_VERTICAL);

        ImageView settingsButton = icon(R.drawable.ic_settings, COLOR_TEXT);
        settingsButton.setPadding(dp(12), dp(12), dp(12), dp(12));
        settingsButton.setFocusable(true);
        settingsButton.setContentDescription("设置");
        settingsButton.setBackground(rounded(COLOR_SURFACE_ALT, COLOR_LINE, 14));
        settingsButton.setOnClickListener(view -> showSettings());
        LinearLayout.LayoutParams settingsParams = new LinearLayout.LayoutParams(dp(46), dp(46));
        settingsParams.leftMargin = dp(10);
        header.addView(settingsButton, settingsParams);
        root.addView(header, matchWrap());
    }

    private void addCapturePanel(LinearLayout root) {
        LinearLayout panel = column();
        panel.setPadding(dp(13), dp(11), dp(12), dp(11));
        panel.setBackground(rounded(COLOR_SURFACE, COLOR_LINE, 8));

        LinearLayout top = row();
        top.setGravity(Gravity.CENTER_VERTICAL);
        TextView heading = text("音频采集", 16, COLOR_TEXT);
        heading.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        top.addView(heading, new LinearLayout.LayoutParams(0, dp(40), 1));
        heading.setGravity(Gravity.CENTER_VERTICAL);

        captureStateText = text("录制已停止", 10, COLOR_MUTED);
        captureStateText.setGravity(Gravity.CENTER);
        captureStateText.setPadding(dp(8), dp(5), dp(8), dp(5));
        captureStateText.setBackground(rounded(COLOR_SURFACE_ALT, COLOR_LINE, 12));
        LinearLayout.LayoutParams stateParams = wrapWrap();
        stateParams.rightMargin = dp(8);
        top.addView(captureStateText, stateParams);
        captureSwitch = new Switch(this);
        styleSwitch(captureSwitch);
        captureSwitch.setContentDescription("音频采集开关");
        captureSwitch.setShowText(false);
        captureSwitch.setChecked(capturing);
        captureSwitch.setOnCheckedChangeListener((button, checked) -> {
            if (checked) {
                capturePending = true;
                updateCaptureUi();
                requestCapture();
            } else if (capturing) {
                sendServiceIntent(new Intent(this, AudioRouterService.class)
                        .setAction(AudioRouterService.ACTION_STOP_CAPTURE));
            }
        });
        top.addView(captureSwitch, wrapWrap());
        panel.addView(top, matchWrap());
        root.addView(panel, matchWrap());
    }

    private void addLanSection(LinearLayout root) {
        LinearLayout section = column();
        section.setBackground(rounded(COLOR_SURFACE, COLOR_LINE, 8));
        LinearLayout controlRow = row();
        controlRow.setGravity(Gravity.CENTER_VERTICAL);
        controlRow.setPadding(dp(12), 0, dp(10), 0);
        TextView label = text("局域网", 17, COLOR_TEXT);
        label.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        label.setGravity(Gravity.CENTER_VERTICAL);
        controlRow.addView(label, new LinearLayout.LayoutParams(0, dp(49), 1));
        lanRefreshButton = createRefreshButton("刷新局域网设备");
        lanRefreshButton.setOnClickListener(view -> refreshLanDevices());
        controlRow.addView(lanRefreshButton, new LinearLayout.LayoutParams(dp(34), dp(34)));
        lanEnableSwitch = new Switch(this);
        styleSwitch(lanEnableSwitch);
        lanEnableSwitch.setShowText(false);
        lanEnableSwitch.setContentDescription("开启局域网");
        lanEnableSwitch.setChecked(settings().getBoolean(PREF_LAN_LISTEN_ENABLED, false));
        lanEnableSwitch.setOnCheckedChangeListener((button, checked) -> {
            if (updatingTransportSwitches) return;
            settings().edit().putBoolean(PREF_LAN_LISTEN_ENABLED, checked).apply();
            sendCommand(AudioRouterService.ACTION_SET_LAN_LISTEN,
                    AudioRouterService.EXTRA_ENABLED, checked);
            latestStatus = checked ? "正在启用局域网发现" : "局域网发现已关闭";
            if (!checked) {
                pendingLanIdentity = "";
                handler.removeCallbacks(lanConnectionTimeout);
            }
            updateTransportVisibility();
            if (checked) refreshLanDevices();
        });
        controlRow.addView(lanEnableSwitch, wrapWrap());
        section.addView(controlRow, matchWrap());

        lanSectionBody = new ExpandableTransportBody(this);
        addDivider(lanSectionBody, 0, 0);
        lanSectionStatus = text("", 11, COLOR_MUTED);
        lanSectionStatus.setPadding(dp(12), dp(9), dp(12), dp(3));
        lanSectionBody.addView(lanSectionStatus, matchWrap());
        lanDeviceList = column();
        LinearLayout.LayoutParams listParams = matchWrap();
        listParams.setMargins(dp(12), dp(2), dp(12), dp(8));
        lanSectionBody.addView(lanDeviceList, listParams);
        section.addView(lanSectionBody, matchWrap());
        root.addView(section, matchWrap());
        updateTransportVisibility();
    }

    private void addBluetoothSection(LinearLayout root) {
        LinearLayout section = column();
        section.setBackground(rounded(COLOR_SURFACE, COLOR_LINE, 8));
        LinearLayout controlRow = row();
        controlRow.setGravity(Gravity.CENTER_VERTICAL);
        controlRow.setPadding(dp(12), 0, dp(10), 0);
        TextView label = text("蓝牙", 17, COLOR_TEXT);
        label.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        label.setGravity(Gravity.CENTER_VERTICAL);
        controlRow.addView(label, new LinearLayout.LayoutParams(0, dp(49), 1));
        bluetoothRefreshButton = createRefreshButton("刷新已配对设备");
        bluetoothRefreshButton.setOnClickListener(view -> refreshDevices());
        controlRow.addView(bluetoothRefreshButton,
                new LinearLayout.LayoutParams(dp(34), dp(34)));
        bluetoothEnableSwitch = new Switch(this);
        styleSwitch(bluetoothEnableSwitch);
        bluetoothEnableSwitch.setShowText(false);
        bluetoothEnableSwitch.setContentDescription("开启蓝牙");
        bluetoothEnableSwitch.setChecked(
                settings().getBoolean(PREF_BLUETOOTH_LISTEN_ENABLED, false));
        bluetoothEnableSwitch.setOnCheckedChangeListener((button, checked) -> {
            if (updatingTransportSwitches) return;
            if (checked && !hasBluetoothPermission()) {
                requestBluetoothPermission();
                return;
            }
            settings().edit().putBoolean(PREF_BLUETOOTH_LISTEN_ENABLED, checked).apply();
            sendCommand(AudioRouterService.ACTION_SET_BLUETOOTH_LISTEN,
                    AudioRouterService.EXTRA_ENABLED, checked);
            latestStatus = checked ? "正在启用蓝牙监听" : "蓝牙监听已关闭";
            if (!checked) {
                pendingBluetoothIdentity = "";
                handler.removeCallbacks(bluetoothConnectionTimeout);
                handler.removeCallbacks(bluetoothRefresh);
            }
            updateTransportVisibility();
            if (checked) refreshDevices();
        });
        controlRow.addView(bluetoothEnableSwitch, wrapWrap());
        section.addView(controlRow, matchWrap());
        bluetoothSectionBody = new ExpandableTransportBody(this);
        addDivider(bluetoothSectionBody, 0, 0);
        bluetoothSectionStatus = text("", 12, COLOR_MUTED);
        bluetoothSectionStatus.setPadding(dp(12), dp(9), dp(12), dp(3));
        bluetoothSectionBody.addView(bluetoothSectionStatus, matchWrap());
        bluetoothDeviceList = column();
        LinearLayout.LayoutParams listParams = matchWrap();
        listParams.setMargins(dp(12), dp(2), dp(12), dp(8));
        bluetoothSectionBody.addView(bluetoothDeviceList, listParams);
        section.addView(bluetoothSectionBody, matchWrap());
        root.addView(section, matchWrap());
        updateTransportVisibility();
    }

    private void renderConnectedCards() {
        renderLanDevices();
        renderBluetoothDevices();
        updateConnectionTimers();
        updateConnectionSummary();
    }

    private void renderLanDevices() {
        if (lanDeviceList == null) return;
        lanDeviceList.removeAllViews();
        lanTimerText = null;
        boolean foundConnected = false;

        if (lanConnected) {
            for (DiscoverySession.Device device : lanDevices) {
                String identity = lanIdentity(device);
                if (identity.equalsIgnoreCase(lanDeviceId)) {
                    addConnectedDeviceRow(lanDeviceList, AudioRouterService.TRANSPORT_LAN,
                            identity, lanName, true);
                    foundConnected = true;
                    break;
                }
            }
            if (!foundConnected) {
                addConnectedDeviceRow(lanDeviceList, AudioRouterService.TRANSPORT_LAN,
                        lanDeviceId, lanName, true);
            }
        }

        for (DiscoverySession.Device device : lanDevices) {
            if (!lanConnected || !lanIdentity(device).equalsIgnoreCase(lanDeviceId)) {
                addLanDeviceRow(device);
            }
        }
        if (lanDeviceList.getChildCount() == 0) {
            lanDeviceList.addView(emptyRow("确认电脑端已启用局域网连接，点击刷新重新搜索"), matchWrap());
        }
    }

    private void renderBluetoothDevices() {
        if (bluetoothDeviceList == null) return;
        bluetoothDeviceList.removeAllViews();
        bluetoothTimerText = null;
        boolean foundConnected = false;

        if (bluetoothConnected) {
            for (BluetoothDevice device : bluetoothDevices) {
                String identity = bluetoothDeviceAddress(device);
                if (bluetoothAddress.equalsIgnoreCase(identity)) {
                    addConnectedDeviceRow(bluetoothDeviceList,
                            AudioRouterService.TRANSPORT_BLUETOOTH,
                            identity, bluetoothName, false);
                    foundConnected = true;
                    break;
                }
            }
            if (!foundConnected) {
                addConnectedDeviceRow(bluetoothDeviceList,
                        AudioRouterService.TRANSPORT_BLUETOOTH,
                        bluetoothAddress, bluetoothName, false);
            }
        }

        for (BluetoothDevice device : bluetoothDevices) {
            if (!bluetoothConnected || !bluetoothAddress.equalsIgnoreCase(
                    bluetoothDeviceAddress(device))) {
                addBluetoothDeviceRow(device);
            }
        }
        if (bluetoothDeviceList.getChildCount() == 0) {
            bluetoothDeviceList.addView(emptyRow("先在系统蓝牙设置中与电脑配对"), matchWrap());
        }
    }

    private String lanIdentity(DiscoverySession.Device device) {
        return device.address.getHostAddress().toLowerCase(java.util.Locale.ROOT) + ":" + device.port;
    }

    private void finishPendingConnection(boolean lan) {
        if (lan) pendingLanIdentity = "";
        else pendingBluetoothIdentity = "";
        renderConnectedCards();
    }

    private void addConnectedDeviceRow(LinearLayout list, String transport, String identity,
                                       String name, boolean lan) {
        LinearLayout entry = column();
        LinearLayout item = row();
        item.setGravity(Gravity.CENTER_VERTICAL);
        item.setPadding(0, dp(8), 0, dp(8));

        TextView deviceName = text(name, 14, COLOR_TEXT);
        deviceName.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        deviceName.setMaxLines(1);
        deviceName.setEllipsize(TextUtils.TruncateAt.END);
        item.addView(deviceName, new LinearLayout.LayoutParams(0,
                ViewGroup.LayoutParams.WRAP_CONTENT, 1));
        TextView detail = text("已连接", 10, COLOR_MINT);
        LinearLayout.LayoutParams detailParams = wrapWrap();
        detailParams.setMargins(dp(6), 0, dp(6), 0);
        item.addView(detail, detailParams);

        TextView timer = text(formatDuration(lan ? lanConnectedAtElapsedRealtime
                : bluetoothConnectedAtElapsedRealtime), 11, COLOR_MINT);
        timer.setGravity(Gravity.CENTER);
        LinearLayout.LayoutParams timerParams = wrapWrap();
        timerParams.rightMargin = dp(8);
        item.addView(timer, timerParams);
        if (lan) lanTimerText = timer;
        else bluetoothTimerText = timer;

        Button disconnect = button("断开", false);
        disconnect.setTextColor(COLOR_BACKGROUND);
        disconnect.setBackground(rounded(COLOR_MINT, COLOR_MINT, 6));
        disconnect.setContentDescription("断开与" + name + "的连接");
        item.addView(disconnect, new LinearLayout.LayoutParams(dp(60), dp(40)));
        disconnect.setOnClickListener(view -> {
            disconnect.setEnabled(false);
            disconnect.setText("断开中");
            sendServiceIntent(new Intent(this,
                AudioRouterService.class)
                .setAction(AudioRouterService.ACTION_DISCONNECT)
                .putExtra(AudioRouterService.EXTRA_TRANSPORT, transport));
        });

        entry.addView(item, matchWrap());
        entry.addView(createDeviceControls(transport, identity), matchWrap());
        list.addView(entry, matchWrap());
        addDivider(list, dp(6), dp(6));
    }

    private LinearLayout createDeviceControls(String transport, String identity) {
        LinearLayout controls = column();
        String currentTransport = transport;
        Switch sendSwitch = createDeviceSwitch(
                "发送音频", "只发送到这台设备",
                readDeviceBoolean(transport, identity, AudioRouterService.SETTING_SEND_ENABLED, true));
        VolumeControl sendVolume = createVolumeControl("发送音量", transport, identity,
                AudioRouterService.ACTION_SEND_VOLUME,
                readDeviceInt(transport, identity, AudioRouterService.SETTING_SEND_VOLUME, 100));
        sendVolume.slider.setEnabled(sendSwitch.isChecked());
        sendVolume.value.setAlpha(sendSwitch.isChecked() ? 1.0f : 0.45f);
        sendSwitch.setOnCheckedChangeListener((button, checked) -> {
            sendVolume.slider.setEnabled(checked);
            sendVolume.value.setAlpha(checked ? 1.0f : 0.45f);
            sendDeviceCommand(currentTransport, identity,
                    AudioRouterService.ACTION_SEND_ENABLED, AudioRouterService.EXTRA_ENABLED, checked);
        });
        controls.addView(sendSwitch, matchWrap());
        controls.addView(sendVolume.root, matchWrap());

        Switch followSwitch = createDeviceSwitch(
                "跟随系统媒体音量", "系统音量变化会实时影响这一路发送",
                readDeviceBoolean(transport, identity, AudioRouterService.SETTING_FOLLOW_SYSTEM_VOLUME,
                        settings().getBoolean(PREF_SYSTEM_VOLUME_ENABLED, false)));
        followSwitch.setOnCheckedChangeListener((button, checked) -> sendDeviceCommand(
                currentTransport, identity, AudioRouterService.ACTION_SYSTEM_VOLUME_ENABLED,
                AudioRouterService.EXTRA_ENABLED, checked));
        LinearLayout.LayoutParams followParams = matchWrap();
        followParams.topMargin = dp(7);
        Switch receiveSwitch = createDeviceSwitch(
                "接收音频", "在手机上播放这台设备传来的声音",
                readDeviceBoolean(transport, identity, AudioRouterService.SETTING_RECEIVE_ENABLED, true));
        VolumeControl receiveVolume = createVolumeControl("接收音量", transport, identity,
                AudioRouterService.ACTION_RECEIVE_VOLUME,
                readDeviceInt(transport, identity, AudioRouterService.SETTING_RECEIVE_VOLUME, 100));
        receiveVolume.slider.setEnabled(receiveSwitch.isChecked());
        receiveVolume.value.setAlpha(receiveSwitch.isChecked() ? 1.0f : 0.45f);
        receiveSwitch.setOnCheckedChangeListener((button, checked) -> {
            receiveVolume.slider.setEnabled(checked);
            receiveVolume.value.setAlpha(checked ? 1.0f : 0.45f);
            sendDeviceCommand(currentTransport, identity, AudioRouterService.ACTION_RECEIVE_ENABLED,
                    AudioRouterService.EXTRA_ENABLED, checked);
        });
        LinearLayout.LayoutParams receiveParams = matchWrap();
        receiveParams.topMargin = dp(1);
        controls.addView(receiveSwitch, receiveParams);
        controls.addView(receiveVolume.root, matchWrap());
        controls.addView(followSwitch, followParams);
        controls.setPadding(dp(4), dp(4), dp(4), dp(3));
        return controls;
    }

    private Switch createDeviceSwitch(String title, String detail, boolean checked) {
        Switch control = new Switch(this);
        styleSwitch(control);
        control.setText(title);
        control.setTextColor(COLOR_TEXT);
        control.setTextSize(13);
        control.setChecked(checked);
        control.setShowText(false);
        control.setContentDescription(title + "。" + detail);
        return control;
    }

    private void styleSwitch(Switch control) {
        control.setGravity(Gravity.CENTER_VERTICAL);
        control.setMinHeight(dp(40));
        int[][] states = new int[][]{
                new int[]{android.R.attr.state_enabled, android.R.attr.state_checked},
                new int[]{android.R.attr.state_enabled, -android.R.attr.state_checked},
                new int[]{-android.R.attr.state_enabled, android.R.attr.state_checked},
                new int[]{-android.R.attr.state_enabled, -android.R.attr.state_checked}
        };
        int checkedThumb = isDarkTheme() ? Color.rgb(13, 32, 27) : Color.WHITE;
        int uncheckedThumb = isDarkTheme() ? COLOR_MUTED : Color.rgb(76, 101, 89);
        int disabledThumb = isDarkTheme() ? Color.rgb(100, 121, 113)
                : Color.rgb(177, 193, 184);
        int checkedTrack = isDarkTheme() ? Color.rgb(111, 199, 169)
                : Color.rgb(33, 127, 96);
        int uncheckedTrack = isDarkTheme() ? Color.rgb(58, 83, 74)
                : Color.rgb(186, 204, 194);
        int disabledTrack = isDarkTheme() ? Color.rgb(45, 62, 56)
                : Color.rgb(218, 228, 222);
        control.setThumbTintList(new ColorStateList(states, new int[]{
                checkedThumb, uncheckedThumb, disabledThumb, disabledThumb
        }));
        control.setTrackTintList(new ColorStateList(states, new int[]{
                checkedTrack, uncheckedTrack, disabledTrack, disabledTrack
        }));
    }

    private VolumeControl createVolumeControl(String title, String transport, String identity,
                                              String action, int initialValue) {
        LinearLayout group = column();
        group.setPadding(dp(4), dp(1), dp(12), dp(4));
        LinearLayout sliderRow = row();
        sliderRow.setGravity(Gravity.CENTER_VERTICAL);
        sliderRow.addView(text(title, 11, COLOR_MUTED),
                new LinearLayout.LayoutParams(dp(56), ViewGroup.LayoutParams.WRAP_CONTENT));
        TextView value = text(initialValue + "%", 11, COLOR_TEXT);
        value.setGravity(Gravity.RIGHT | Gravity.CENTER_VERTICAL);

        SeekBar slider = new SeekBar(this);
        slider.setMax(100);
        slider.setProgress(initialValue);
        slider.setPadding(dp(10), 0, dp(10), 0);
        slider.setProgressTintList(ColorStateList.valueOf(COLOR_MINT));
        slider.setThumbTintList(ColorStateList.valueOf(COLOR_MINT));
        slider.setProgressBackgroundTintList(ColorStateList.valueOf(COLOR_LINE));
        slider.setContentDescription(title);
        slider.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
            @Override public void onProgressChanged(SeekBar bar, int progress, boolean fromUser) {
                value.setText(progress + "%");
                if (fromUser) {
                    sendDeviceCommand(transport, identity, action,
                            AudioRouterService.EXTRA_VOLUME, progress);
                }
            }
            @Override public void onStartTrackingTouch(SeekBar bar) { }
            @Override public void onStopTrackingTouch(SeekBar bar) { }
        });
        LinearLayout.LayoutParams sliderParams = new LinearLayout.LayoutParams(0, dp(40), 1);
        sliderParams.rightMargin = dp(4);
        sliderRow.addView(slider, sliderParams);
        sliderRow.addView(value, new LinearLayout.LayoutParams(dp(36),
                ViewGroup.LayoutParams.WRAP_CONTENT));
        group.addView(sliderRow, matchWrap());
        return new VolumeControl(group, slider, value);
    }

    private void refreshLanDevices() {
        if (lanDeviceList == null) return;
        if (discoverySession != null) discoverySession.close();
        discoverySession = null;
        if (!lanEnableSwitch.isChecked()) {
            setRefreshing(false, true);
            return;
        }
        discoverySession = new DiscoverySession();
        setRefreshing(true, true);
        lanSectionStatus.setText("正在扫描同一网络中的 AudioRouter 设备…");
        DiscoverySession current = discoverySession;
        current.scan(new DiscoverySession.Listener() {
            @Override public void onDevices(List<DiscoverySession.Device> devices) {
                runOnUiThread(() -> {
                    if (isDestroyed() || discoverySession != current) return;
                    setRefreshing(false, true);
                    lanDevices.clear();
                    lanDevices.addAll(devices);
                    lanSectionStatus.setText(devices.isEmpty()
                            ? "局域网内的设备 · 未发现设备"
                            : "局域网内的设备 · " + devices.size() + " 台");
                    renderLanDevices();
                });
            }
            @Override public void onError(String message) {
                runOnUiThread(() -> {
                    if (!isDestroyed() && discoverySession == current) {
                        setRefreshing(false, true);
                        lanSectionStatus.setText(message);
                        renderLanDevices();
                    }
                });
            }
        });
    }

    private void addLanDeviceRow(DiscoverySession.Device device) {
        String identity = lanIdentity(device);
        if (lanConnected && identity.equalsIgnoreCase(lanDeviceId)) {
            addConnectedDeviceRow(lanDeviceList, AudioRouterService.TRANSPORT_LAN,
                    identity, lanName, true);
            return;
        }
        LinearLayout entry = column();
        String address = settings().getBoolean(PREF_SHOW_IP_ADDRESS, false)
                ? device.address.getHostAddress() : "";
        LinearLayout item = deviceRow(device.name, address);
        Button connect = button("连接", false);
        LinearLayout.LayoutParams buttonParams = new LinearLayout.LayoutParams(dp(60), dp(40));
        buttonParams.leftMargin = dp(8);
        item.addView(connect, buttonParams);
        connect.setEnabled(lanEnableSwitch.isChecked() && pendingLanIdentity.isEmpty());
        if (identity.equalsIgnoreCase(pendingLanIdentity)) connect.setText("连接中");
        connect.setContentDescription("连接局域网设备" + device.name);
        connect.setOnClickListener(view -> {
            if (!lanEnableSwitch.isChecked()) return;
            pendingLanIdentity = identity;
            renderLanDevices();
            handler.removeCallbacks(lanConnectionTimeout);
            handler.postDelayed(lanConnectionTimeout, 12000);
            sendServiceIntent(new Intent(this, AudioRouterService.class)
                    .setAction(AudioRouterService.ACTION_CONNECT)
                    .putExtra(AudioRouterService.EXTRA_HOST, device.address.getHostAddress())
                    .putExtra(AudioRouterService.EXTRA_PORT, device.port)
                    .putExtra(AudioRouterService.EXTRA_DEVICE_NAME, device.name));
            latestStatus = "正在连接 " + device.name;
            lanSectionStatus.setText(latestStatus);
            updateConnectionSummary();
        });
        entry.addView(item, matchWrap());
        if (settings().getBoolean(PREF_PRECONNECT_CONTROLS, false)) {
            entry.addView(createDeviceControls(AudioRouterService.TRANSPORT_LAN, identity), matchWrap());
        }
        lanDeviceList.addView(entry, matchWrap());
        addDivider(lanDeviceList, dp(6), dp(6));
    }

    private void refreshDevices() {
        if (bluetoothDeviceList == null) return;
        handler.removeCallbacks(bluetoothRefresh);
        if (!bluetoothEnableSwitch.isChecked()) {
            setRefreshing(false, false);
            return;
        }
        setRefreshing(true, false);
        bluetoothSectionStatus.setText("正在读取已配对设备…");
        handler.postDelayed(bluetoothRefresh, 400);
    }

    private void refreshBluetoothDevicesNow() {
        if (!bluetoothEnableSwitch.isChecked() || bluetoothDeviceList == null) {
            setRefreshing(false, false);
            return;
        }
        if (Build.VERSION.SDK_INT >= 31
                && checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT)
                != PackageManager.PERMISSION_GRANTED) {
            setRefreshing(false, false);
            bluetoothSectionStatus.setText("需要蓝牙访问权限");
            requestBluetoothPermission();
            return;
        }
        try {
            BluetoothAdapter adapter = BluetoothAdapter.getDefaultAdapter();
            if (adapter == null) {
                setRefreshing(false, false);
                bluetoothSectionStatus.setText("此设备不支持蓝牙");
                bluetoothDeviceList.addView(emptyRow("蓝牙不可用"), matchWrap());
                return;
            }
            if (!adapter.isEnabled()) {
                setRefreshing(false, false);
                bluetoothSectionStatus.setText("等待系统蓝牙授权");
                try {
                    startActivityForResult(new Intent(BluetoothAdapter.ACTION_REQUEST_ENABLE),
                            REQUEST_ENABLE_BLUETOOTH);
                } catch (RuntimeException error) {
                    bluetoothSectionStatus.setText("请在系统设置中打开蓝牙");
                }
                return;
            }
            Set<BluetoothDevice> bonded = adapter.getBondedDevices();
            List<BluetoothDevice> devices = new ArrayList<>(bonded);
            devices.sort((first, second) -> bluetoothDeviceAddress(first)
                    .compareTo(bluetoothDeviceAddress(second)));
            bluetoothDevices.clear();
            bluetoothDevices.addAll(devices);
            setRefreshing(false, false);
            bluetoothSectionStatus.setText(devices.isEmpty()
                    ? "已配对设备 · 未发现其它设备"
                    : "已配对设备 · " + devices.size() + " 台");
            renderBluetoothDevices();
        } catch (SecurityException error) {
            setRefreshing(false, false);
            bluetoothSectionStatus.setText("没有蓝牙访问权限");
        }
    }

    private void addBluetoothDeviceRow(BluetoothDevice device) {
        String name;
        try {
            name = device.getName();
        } catch (SecurityException error) {
            name = "蓝牙设备";
        }
        if (name == null || name.trim().isEmpty()) name = "未命名设备";
        String identity = bluetoothDeviceAddress(device);
        if (bluetoothConnected && !bluetoothAddress.isEmpty()
                && bluetoothAddress.equalsIgnoreCase(identity)) {
            addConnectedDeviceRow(bluetoothDeviceList, AudioRouterService.TRANSPORT_BLUETOOTH,
                    identity, bluetoothName, false);
            return;
        }
        LinearLayout entry = column();
        LinearLayout item = deviceRow(name, "已配对");
        Button connect = button("连接", false);
        LinearLayout.LayoutParams buttonParams = new LinearLayout.LayoutParams(dp(60), dp(40));
        buttonParams.leftMargin = dp(8);
        item.addView(connect, buttonParams);
        connect.setEnabled(bluetoothEnableSwitch.isChecked() && pendingBluetoothIdentity.isEmpty());
        if (identity.equalsIgnoreCase(pendingBluetoothIdentity)) connect.setText("连接中");
        connect.setContentDescription("连接蓝牙设备" + name);
        connect.setOnClickListener(view -> {
            pendingBluetoothIdentity = identity;
            renderBluetoothDevices();
            handler.removeCallbacks(bluetoothConnectionTimeout);
            handler.postDelayed(bluetoothConnectionTimeout, 15000);
            connectBluetoothDevice(device);
        });
        entry.addView(item, matchWrap());
        if (settings().getBoolean(PREF_PRECONNECT_CONTROLS, false)) {
            entry.addView(createDeviceControls(AudioRouterService.TRANSPORT_BLUETOOTH, identity), matchWrap());
        }
        bluetoothDeviceList.addView(entry, matchWrap());
        addDivider(bluetoothDeviceList, dp(6), dp(6));
    }

    private void connectBluetoothDevice(BluetoothDevice device) {
        if (!bluetoothEnableSwitch.isChecked()) return;
        try {
            sendServiceIntent(new Intent(this, AudioRouterService.class)
                    .setAction(AudioRouterService.ACTION_BLUETOOTH_CONNECT)
                    .putExtra(AudioRouterService.EXTRA_DEVICE, device)
                    .putExtra(AudioRouterService.EXTRA_DEVICE_NAME, bluetoothDeviceLabel(device)));
            String name;
            try { name = device.getName(); } catch (SecurityException error) { name = "蓝牙设备"; }
            latestStatus = "正在连接 " + friendlyName(name);
            bluetoothSectionStatus.setText(latestStatus);
            updateConnectionSummary();
        } catch (SecurityException error) {
            latestStatus = "蓝牙连接权限不足";
            updateConnectionSummary();
        }
    }

    private String bluetoothDeviceAddress(BluetoothDevice device) {
        try {
            String address = device.getAddress();
            return address == null ? "" : address;
        } catch (SecurityException error) {
            return "";
        }
    }

    private String bluetoothDeviceLabel(BluetoothDevice device) {
        try {
            String name = device.getName();
            return name == null || name.trim().isEmpty() ? "未命名设备" : name;
        } catch (SecurityException error) {
            return "蓝牙设备";
        }
    }

    private void requestCapture() {
        boolean notificationPermission = Build.VERSION.SDK_INT < 33
                || checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                == PackageManager.PERMISSION_GRANTED;
        if (!notificationPermission) {
            new AlertDialog.Builder(this)
                    .setTitle("允许显示音频采集通知？")
                    .setMessage("通知用于显示后台音频采集状态。应用只采集设备播放音频，不使用麦克风。")
                    .setNegativeButton("暂不允许", (dialog, which) -> cancelCaptureRequest())
                    .setPositiveButton("继续", (dialog, which) -> requestPermissions(
                            new String[]{Manifest.permission.POST_NOTIFICATIONS}, REQUEST_PERMISSIONS))
                    .setOnCancelListener(dialog -> cancelCaptureRequest()).show();
            return;
        }
        try {
            MediaProjectionManager manager = getSystemService(MediaProjectionManager.class);
            startActivityForResult(manager.createScreenCaptureIntent(), REQUEST_CAPTURE);
        } catch (RuntimeException error) {
            cancelCaptureRequest();
            latestStatus = "无法请求系统播放音频捕获授权";
            Toast.makeText(this, latestStatus, Toast.LENGTH_LONG).show();
            updateConnectionSummary();
        }
    }

    private void cancelCaptureRequest() {
        capturePending = false;
        handler.removeCallbacks(captureStartTimeout);
        updateCaptureUi();
    }

    private boolean hasBluetoothPermission() {
        return Build.VERSION.SDK_INT < 31 || checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT)
                == PackageManager.PERMISSION_GRANTED;
    }

    private void requestBluetoothPermission() {
        if (bluetoothPermissionPending) return;
        bluetoothPermissionPending = true;
        bluetoothEnableSwitch.setChecked(false);
        new AlertDialog.Builder(this)
                .setTitle("允许 AudioRouter 使用附近设备？")
                .setMessage("应用需要蓝牙访问权限来读取已配对设备并建立连接。不会使用麦克风。")
                .setNegativeButton("暂不允许", (dialog, which) -> bluetoothPermissionPending = false)
                .setPositiveButton("允许", (dialog, which) -> requestPermissions(
                        new String[]{Manifest.permission.BLUETOOTH_CONNECT}, REQUEST_BLUETOOTH))
                .setOnCancelListener(dialog -> bluetoothPermissionPending = false).show();
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == REQUEST_ENABLE_BLUETOOTH) {
            if (resultCode == RESULT_OK) {
                if (bluetoothEnableSwitch.isChecked()) {
                    sendCommand(AudioRouterService.ACTION_SET_BLUETOOTH_LISTEN,
                            AudioRouterService.EXTRA_ENABLED, true);
                    refreshDevices();
                }
            } else if (bluetoothSectionStatus != null) {
                bluetoothEnableSwitch.setChecked(false);
                bluetoothSectionStatus.setText("系统蓝牙未开启");
            }
        } else if (requestCode == REQUEST_CAPTURE && resultCode == RESULT_OK && data != null) {
            capturePending = true;
            updateCaptureUi();
            handler.removeCallbacks(captureStartTimeout);
            handler.postDelayed(captureStartTimeout, 8000);
            sendServiceIntent(new Intent(this, AudioRouterService.class)
                    .setAction(AudioRouterService.ACTION_CAPTURE)
                    .putExtra(AudioRouterService.EXTRA_RESULT_CODE, resultCode)
                    .putExtra(AudioRouterService.EXTRA_RESULT_DATA, data));
        } else if (requestCode == REQUEST_CAPTURE) {
            cancelCaptureRequest();
        }
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, String[] permissions,
                                           int[] grantResults) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults);
        if (requestCode == REQUEST_BLUETOOTH) {
            bluetoothPermissionPending = false;
            if (grantResults.length > 0 && grantResults[0] == PackageManager.PERMISSION_GRANTED) {
                bluetoothEnableSwitch.setChecked(true);
            } else {
                bluetoothSectionStatus.setText("需要蓝牙访问权限才能读取已配对设备");
            }
        } else if (requestCode == REQUEST_PERMISSIONS) {
            boolean granted = grantResults.length > 0;
            for (int result : grantResults) granted &= result == PackageManager.PERMISSION_GRANTED;
            if (granted) {
                requestCapture();
            } else {
                cancelCaptureRequest();
                latestStatus = "需要通知权限才能显示音频采集状态";
                updateConnectionSummary();
            }
        }
    }

    private void updateCaptureUi() {
        updateAudioSettingsLockAppearance();
        if (captureStateText == null || captureSwitch == null) return;
        captureSwitch.setEnabled(!capturePending);
        boolean checkedState = capturing || capturePending;
        if (captureSwitch.isChecked() != checkedState) {
            captureSwitch.setOnCheckedChangeListener(null);
            captureSwitch.setChecked(checkedState);
            captureSwitch.setOnCheckedChangeListener((button, checked) -> {
                if (checked) {
                    capturePending = true;
                    updateCaptureUi();
                    requestCapture();
                } else if (capturing) {
                    sendServiceIntent(new Intent(this, AudioRouterService.class)
                            .setAction(AudioRouterService.ACTION_STOP_CAPTURE));
                }
            });
        }
        captureStateText.setText(capturePending ? "等待授权" : capturing ? "录制进行中" : "录制已停止");
        captureSwitch.setContentDescription(capturing ? "停止手机音频采集" : "开始手机音频采集");
        captureStateText.setTextColor(capturing ? COLOR_MINT : COLOR_MUTED);
        captureStateText.setBackground(rounded(
                capturing ? COLOR_SURFACE_ALT : COLOR_SURFACE_ALT,
                capturing ? COLOR_MINT : COLOR_LINE, 12));
    }

    private void updateConnectionSummary() {
        updateCaptureUi();
    }

    private void updateConnectionTimers() {
        if (lanTimerText != null) {
            lanTimerText.setText(formatDuration(lanConnectedAtElapsedRealtime));
        }
        if (bluetoothTimerText != null) {
            bluetoothTimerText.setText(formatDuration(bluetoothConnectedAtElapsedRealtime));
        }
    }

    private String formatDuration(long connectedAtElapsedRealtime) {
        if (connectedAtElapsedRealtime <= 0) return "00:00";
        long seconds = Math.max(0, SystemClock.elapsedRealtime() - connectedAtElapsedRealtime) / 1000;
        long hours = seconds / 3600;
        long minutes = (seconds / 60) % 60;
        long remainder = seconds % 60;
        return hours > 0
                ? String.format(java.util.Locale.ROOT, "%02d:%02d:%02d", hours, minutes, remainder)
                : String.format(java.util.Locale.ROOT, "%02d:%02d", minutes, remainder);
    }

    private void sendDeviceCommand(String transport, String identity,
                                   String action, String key, boolean value) {
        String setting = AudioRouterService.ACTION_SEND_ENABLED.equals(action)
                ? AudioRouterService.SETTING_SEND_ENABLED
                : AudioRouterService.ACTION_RECEIVE_ENABLED.equals(action)
                ? AudioRouterService.SETTING_RECEIVE_ENABLED : AudioRouterService.SETTING_FOLLOW_SYSTEM_VOLUME;
        settings().edit().putBoolean(AudioRouterService.devicePreferenceKey(transport, identity, setting),
                value).apply();
        sendServiceIntent(new Intent(this, AudioRouterService.class)
                .setAction(action)
                .putExtra(AudioRouterService.EXTRA_TRANSPORT, transport)
                .putExtra(AudioRouterService.EXTRA_DEVICE_ID, identity)
                .putExtra(key, value));
    }

    private void sendDeviceCommand(String transport, String identity,
                                   String action, String key, int value) {
        String setting = AudioRouterService.ACTION_SEND_VOLUME.equals(action)
                ? AudioRouterService.SETTING_SEND_VOLUME : AudioRouterService.SETTING_RECEIVE_VOLUME;
        settings().edit().putInt(AudioRouterService.devicePreferenceKey(transport, identity, setting),
                value).apply();
        sendServiceIntent(new Intent(this, AudioRouterService.class)
                .setAction(action)
                .putExtra(AudioRouterService.EXTRA_TRANSPORT, transport)
                .putExtra(AudioRouterService.EXTRA_DEVICE_ID, identity)
                .putExtra(key, value));
    }

    private void sendCommand(String action, String key, boolean value) {
        sendServiceIntent(new Intent(this, AudioRouterService.class)
                .setAction(action).putExtra(key, value));
    }

    private void sendServiceIntent(Intent intent) {
        try {
            String action = intent.getAction();
            boolean needsForegroundStart = AudioRouterService.ACTION_LISTEN.equals(action)
                    || AudioRouterService.ACTION_SET_LAN_LISTEN.equals(action)
                    || AudioRouterService.ACTION_SET_BLUETOOTH_LISTEN.equals(action)
                    || AudioRouterService.ACTION_SET_LAN_AUDIO_QUALITY.equals(action)
                    || AudioRouterService.ACTION_SET_BLUETOOTH_AUDIO_QUALITY.equals(action)
                    || AudioRouterService.ACTION_CONNECT.equals(action)
                    || AudioRouterService.ACTION_BLUETOOTH_CONNECT.equals(action)
                    || AudioRouterService.ACTION_CAPTURE.equals(action);
            if (needsForegroundStart && Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                startForegroundService(intent);
            } else {
                startService(intent);
            }
        } catch (RuntimeException error) {
            capturePending = false;
            pendingLanIdentity = "";
            pendingBluetoothIdentity = "";
            latestStatus = "AudioRouter 服务启动失败";
            Toast.makeText(this, latestStatus, Toast.LENGTH_LONG).show();
            updateConnectionSummary();
        }
    }

    private boolean readDeviceBoolean(String transport, String identity, String setting, boolean fallback) {
        return settings().getBoolean(
                AudioRouterService.devicePreferenceKey(transport, identity, setting), fallback);
    }

    private int readDeviceInt(String transport, String identity, String setting, int fallback) {
        return settings().getInt(
                AudioRouterService.devicePreferenceKey(transport, identity, setting), fallback);
    }

    private android.content.SharedPreferences settings() {
        return getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
    }

    private LinearLayout deviceRow(String name, String detail) {
        LinearLayout item = row();
        item.setGravity(Gravity.CENTER_VERTICAL);
        TextView information = text(name, 14, COLOR_TEXT);
        information.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        information.setMaxLines(1);
        information.setEllipsize(TextUtils.TruncateAt.END);
        TextView detailText = text(detail, 10, COLOR_MUTED);
        if (detail == null || detail.isEmpty()) detailText.setVisibility(View.GONE);
        detailText.setMaxLines(1);
        detailText.setEllipsize(TextUtils.TruncateAt.END);
        item.addView(information, new LinearLayout.LayoutParams(0,
                ViewGroup.LayoutParams.WRAP_CONTENT, 1));
        LinearLayout.LayoutParams detailParams = new LinearLayout.LayoutParams(dp(92),
                ViewGroup.LayoutParams.WRAP_CONTENT);
        detailParams.leftMargin = dp(6);
        item.addView(detailText, detailParams);
        item.setPadding(0, dp(9), 0, dp(9));
        return item;
    }

    private TextView emptyRow(String value) {
        TextView empty = text(value, 12, COLOR_MUTED);
        empty.setPadding(0, dp(14), 0, dp(14));
        return empty;
    }

    private Button button(String label, boolean primary) {
        Button button = new Button(this);
        button.setText(label);
        button.setTextSize(11);
        button.setAllCaps(false);
        button.setMinHeight(0);
        button.setMinimumHeight(0);
        button.setMinWidth(0);
        button.setMinimumWidth(0);
        button.setPadding(dp(8), dp(4), dp(8), dp(4));
        button.setTextColor(primary ? COLOR_BACKGROUND : COLOR_MINT);
        button.setBackground(rounded(primary ? COLOR_AMBER : COLOR_SURFACE_ALT,
                primary ? COLOR_AMBER : COLOR_LINE, 10));
        return button;
    }

    private void addDivider(LinearLayout parent, int top, int bottom) {
        View line = new View(this);
        line.setBackgroundColor(COLOR_LINE);
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, dp(1));
        params.topMargin = top;
        params.bottomMargin = bottom;
        parent.addView(line, params);
    }

    private void addSpace(LinearLayout parent, int heightDp) {
        View space = new View(this);
        parent.addView(space, new LinearLayout.LayoutParams(1, dp(heightDp)));
    }

    private TextView text(String value, int sizeSp, int color) {
        TextView text = new TextView(this);
        text.setText(value);
        text.setTextSize(sizeSp);
        text.setTextColor(color);
        return text;
    }

    private ImageView icon(int resource, int color) {
        ImageView view = new ImageView(this);
        view.setImageResource(resource);
        view.setImageTintList(ColorStateList.valueOf(color));
        view.setScaleType(ImageView.ScaleType.CENTER_INSIDE);
        return view;
    }

    private LinearLayout column() {
        LinearLayout layout = new LinearLayout(this);
        layout.setOrientation(LinearLayout.VERTICAL);
        return layout;
    }

    private LinearLayout row() {
        LinearLayout layout = new LinearLayout(this);
        layout.setOrientation(LinearLayout.HORIZONTAL);
        return layout;
    }

    private GradientDrawable rounded(int fill, int stroke, int radiusDp) {
        GradientDrawable drawable = new GradientDrawable();
        drawable.setColor(fill);
        drawable.setCornerRadius(dp(radiusDp));
        if (stroke != Color.TRANSPARENT) drawable.setStroke(dp(1), stroke);
        return drawable;
    }

    private LinearLayout.LayoutParams matchWrap() {
        return new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
    }

    private LinearLayout.LayoutParams wrapWrap() {
        return new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT);
    }

    private int dp(float value) {
        return Math.round(value * getResources().getDisplayMetrics().density);
    }

    private String friendlyName(String value) {
        return value == null || value.trim().isEmpty() ? "蓝牙设备" : value.trim();
    }

    private static final class ExpandableTransportBody extends LinearLayout {
        private boolean expanded = true;
        private int contentHeight;
        private int revealHeight = -1;
        private ValueAnimator heightAnimator;

        ExpandableTransportBody(Context context) {
            super(context);
            setOrientation(VERTICAL);
            setClipChildren(true);
            setClipToPadding(true);
        }

        @Override protected void onMeasure(int widthMeasureSpec, int heightMeasureSpec) {
            super.onMeasure(widthMeasureSpec, MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED));
            contentHeight = getMeasuredHeight();
            if (revealHeight >= 0) setMeasuredDimension(getMeasuredWidth(), revealHeight);
        }

        void setExpanded(boolean nextExpanded) {
            if (expanded == nextExpanded) return;
            int startHeight = getVisibility() == GONE ? 0 : getHeight();
            expanded = nextExpanded;
            cancelHeightAnimation();
            ViewGroup parent = (ViewGroup) getParent();
            if (!ValueAnimator.areAnimatorsEnabled() || !isAttachedToWindow()
                    || parent == null || !parent.isLaidOut()) {
                settleExpansion();
                return;
            }
            int width = parent.getWidth() - parent.getPaddingLeft() - parent.getPaddingRight();
            ViewGroup.LayoutParams params = getLayoutParams();
            if (params instanceof ViewGroup.MarginLayoutParams) {
                ViewGroup.MarginLayoutParams margins = (ViewGroup.MarginLayoutParams) params;
                width -= margins.leftMargin + margins.rightMargin;
            }
            if (width <= 0) {
                settleExpansion();
                return;
            }
            measure(MeasureSpec.makeMeasureSpec(width, MeasureSpec.EXACTLY),
                    MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED));
            int targetHeight = expanded ? contentHeight : 0;
            if (startHeight == targetHeight) {
                settleExpansion();
                return;
            }
            revealHeight = startHeight;
            setVisibility(VISIBLE);
            ValueAnimator animator = ValueAnimator.ofFloat(0f, 1f);
            heightAnimator = animator;
            animator.setDuration(220);
            animator.setInterpolator(new DecelerateInterpolator(2f));
            animator.addUpdateListener(animation -> {
                float progress = (float) animation.getAnimatedValue();
                int endHeight = expanded ? contentHeight : 0;
                revealHeight = Math.round(startHeight + (endHeight - startHeight) * progress);
                requestLayout();
            });
            animator.addListener(new AnimatorListenerAdapter() {
                @Override public void onAnimationEnd(Animator animation) {
                    if (heightAnimator != animation) return;
                    heightAnimator = null;
                    settleExpansion();
                }
            });
            requestLayout();
            animator.start();
        }

        private void cancelHeightAnimation() {
            ValueAnimator previous = heightAnimator;
            heightAnimator = null;
            if (previous != null) previous.cancel();
        }

        private void settleExpansion() {
            revealHeight = -1;
            setVisibility(expanded ? VISIBLE : GONE);
            requestLayout();
        }

        @Override protected void onDetachedFromWindow() {
            cancelHeightAnimation();
            settleExpansion();
            super.onDetachedFromWindow();
        }
    }

    private static final class VolumeControl {
        final LinearLayout root;
        final SeekBar slider;
        final TextView value;

        VolumeControl(LinearLayout root, SeekBar slider, TextView value) {
            this.root = root;
            this.slider = slider;
            this.value = value;
        }
    }

    private final class RefreshControl extends FrameLayout {
        final ImageView glyph;

        RefreshControl(Context context) {
            super(context);
            setClipChildren(false);
            glyph = new ImageView(context);
            glyph.setImageResource(R.drawable.ic_sync);
            glyph.setImageTintList(ColorStateList.valueOf(COLOR_MINT));
            glyph.setScaleType(ImageView.ScaleType.CENTER_INSIDE);
            addView(glyph, new FrameLayout.LayoutParams(dp(20), dp(20), Gravity.CENTER));
        }
    }

}
