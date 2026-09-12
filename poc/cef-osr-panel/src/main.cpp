#include "include/cef_app.h"
#include "include/cef_browser.h"
#include "include/cef_client.h"
#include "include/cef_render_handler.h"
#include "include/wrapper/cef_helpers.h"

#include <X11/Xlib.h>
#include <X11/keysym.h>
#include <X11/Xutil.h>

#include <algorithm>
#include <atomic>
#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <charconv>
#include <filesystem>
#include <iostream>
#include <mutex>
#include <optional>
#include <string>
#include <thread>
#include <vector>

#include <limits.h>
#include <unistd.h>
namespace {

constexpr int kDefaultWidth = CEF_OSR_PANEL_DEFAULT_WIDTH;
constexpr int kDefaultHeight = CEF_OSR_PANEL_DEFAULT_HEIGHT;
constexpr int kDefaultRemoteDebuggingPort = 9223;

std::atomic<bool> g_done{false};
std::atomic<bool> g_close_requested{false};

struct Options {
  std::string fixture_url;
  std::string cache_path = "runtime/cache";
  int remote_debugging_port = kDefaultRemoteDebuggingPort;
};

std::optional<std::string> ArgumentValue(const CefMainArgs& args, const std::string& prefix) {
  for (int i = 1; i < args.argc; ++i) {
    const std::string argument(args.argv[i]);
    if (argument.rfind(prefix, 0) == 0) {
      return argument.substr(prefix.size());
    }
  }
  return std::nullopt;
}

Options ParseOptions(const CefMainArgs& args) {
  Options options;
  if (const auto fixture = ArgumentValue(args, "--fixture=")) {
    options.fixture_url = *fixture;
  }
  if (const auto cache = ArgumentValue(args, "--cache-path=")) {
    options.cache_path = *cache;
  }
  if (const auto port = ArgumentValue(args, "--remote-debugging-port=")) {
    int value = 0;
    const auto parsed = std::from_chars(port->data(), port->data() + port->size(), value);
    if (parsed.ec == std::errc{} && parsed.ptr == port->data() + port->size() &&
        value > 0 && value <= 65535) {
      options.remote_debugging_port = value;
    }
  }
  return options;
}

std::string FixtureUrl(const Options& options) {
  if (!options.fixture_url.empty()) {
    return options.fixture_url;
  }
  const auto path = std::filesystem::absolute("fixture/index.html");
  return "file://" + path.string();
}


std::filesystem::path ExecutableDirectory() {
  char executable_path[PATH_MAX] = {};
  const ssize_t length = readlink("/proc/self/exe", executable_path, sizeof(executable_path) - 1);
  if (length > 0) {
    executable_path[length] = '\0';
    return std::filesystem::path(executable_path).parent_path();
  }
  return std::filesystem::current_path();
}
class HostWindow;
class PanelClient;

class PanelRenderHandler final : public CefRenderHandler {
 public:
  explicit PanelRenderHandler(HostWindow* host) : host_(host) {}

  void GetViewRect(CefRefPtr<CefBrowser> browser, CefRect& rect) override;
  void OnPaint(CefRefPtr<CefBrowser> browser,
               PaintElementType type,
               const RectList& dirty_rects,
               const void* buffer,
               int width,
               int height) override;

 private:
  HostWindow* host_;
  IMPLEMENT_REFCOUNTING(PanelRenderHandler);
};

class HostWindow {
 public:
  HostWindow() = default;
  ~HostWindow() {
    if (image_ != nullptr) {
      XDestroyImage(image_);
      image_ = nullptr;
    }
    if (display_ != nullptr && window_ != 0) {
      XDestroyWindow(display_, window_);
    }
    if (display_ != nullptr) {
      XCloseDisplay(display_);
    }
  }

  bool Create(int width, int height) {
    width_ = std::max(1, width);
    height_ = std::max(1, height);
    display_ = XOpenDisplay(nullptr);
    if (display_ == nullptr) {
      std::cerr << "CEF OSR POC: cannot open the X11 display" << std::endl;
      return false;
    }

    const int screen = DefaultScreen(display_);
    window_ = XCreateSimpleWindow(display_, RootWindow(display_, screen), 0, 0,
                                  static_cast<unsigned>(width_),
                                  static_cast<unsigned>(height_), 0,
                                  BlackPixel(display_, screen),
                                  WhitePixel(display_, screen));
    if (window_ == 0) {
      std::cerr << "CEF OSR POC: cannot create the X11 host window" << std::endl;
      return false;
    }

    XStoreName(display_, window_, "CEF OSR/CDP panel POC");
    XSelectInput(display_, window_, ExposureMask | StructureNotifyMask |
                                      KeyPressMask | ButtonPressMask |
                                      ButtonReleaseMask | PointerMotionMask);
    delete_atom_ = XInternAtom(display_, "WM_DELETE_WINDOW", False);
    XSetWMProtocols(display_, window_, &delete_atom_, 1);
    XMapWindow(display_, window_);
    XFlush(display_);
    return true;
  }

  CefWindowHandle window_handle() const { return window_; }

  void SetBrowser(CefRefPtr<CefBrowser> browser) {
    std::lock_guard<std::mutex> lock(browser_mutex_);
    browser_ = browser;
  }

  CefRefPtr<CefBrowser> browser() {
    std::lock_guard<std::mutex> lock(browser_mutex_);
    return browser_;
  }

  void ClearBrowser() {
    std::lock_guard<std::mutex> lock(browser_mutex_);
    browser_ = nullptr;
  }

  void Configure(int width, int height) {
    width_ = std::max(1, width);
    height_ = std::max(1, height);
    if (auto browser_ref = browser()) {
      browser_ref->GetHost()->WasResized();
    }
  }

  bool IsClosing() const { return g_close_requested.load(); }

  void RequestClose() {
    if (g_close_requested.exchange(true)) {
      return;
    }
    if (auto browser_ref = browser()) {
      browser_ref->GetHost()->CloseBrowser(false);
    } else {
      g_done.store(true);
    }
  }

  void SubmitFrame(const void* buffer, int width, int height) {
    if (buffer == nullptr || width <= 0 || height <= 0) {
      return;
    }
    std::lock_guard<std::mutex> lock(frame_mutex_);
    frame_.assign(static_cast<const uint8_t*>(buffer),
                  static_cast<const uint8_t*>(buffer) +
                      static_cast<size_t>(width) * static_cast<size_t>(height) * 4);
    frame_width_ = width;
    frame_height_ = height;
    ++frame_sequence_;
    frame_ready_ = true;
  }

  void PresentLatest() {
    std::vector<uint8_t> frame;
    int width = 0;
    int height = 0;
    uint64_t sequence = 0;
    {
      std::lock_guard<std::mutex> lock(frame_mutex_);
      if (!frame_ready_) {
        return;
      }
      frame = frame_;
      width = frame_width_;
      height = frame_height_;
      sequence = frame_sequence_;
      frame_ready_ = false;
    }

    EnsureImage(width, height);
    if (image_ == nullptr) {
      return;
    }
    const unsigned long red_mask = image_->red_mask;
    const unsigned long green_mask = image_->green_mask;
    const unsigned long blue_mask = image_->blue_mask;
    const int red_shift = LowestBit(red_mask);
    const int green_shift = LowestBit(green_mask);
    const int blue_shift = LowestBit(blue_mask);

    for (int y = 0; y < height; ++y) {
      for (int x = 0; x < width; ++x) {
        const size_t offset = (static_cast<size_t>(y) * width + x) * 4;
        const unsigned long red = frame[offset + 2];
        const unsigned long green = frame[offset + 1];
        const unsigned long blue = frame[offset];
        const unsigned long pixel = ((red << red_shift) & red_mask) |
                                    ((green << green_shift) & green_mask) |
                                    ((blue << blue_shift) & blue_mask);
        XPutPixel(image_, x, y, pixel);
      }
    }
    XPutImage(display_, window_, DefaultGC(display_, DefaultScreen(display_)), image_,
              0, 0, 0, 0, static_cast<unsigned>(width), static_cast<unsigned>(height));
    XFlush(display_);

    uint64_t checksum = 1469598103934665603ULL;
    for (const uint8_t byte : frame) {
      checksum ^= byte;
      checksum *= 1099511628211ULL;
    }
    std::cout << "FRAME sequence=" << sequence << " width=" << width
              << " height=" << height << " checksum=" << checksum << std::endl;
  }

  void DispatchEvents() {
    while (display_ != nullptr && XPending(display_) > 0) {
      XEvent event;
      XNextEvent(display_, &event);
      auto browser_ref = browser();
      if (event.type == ClientMessage &&
          static_cast<Atom>(event.xclient.data.l[0]) == delete_atom_) {
        RequestClose();
        continue;
      }
      if (event.type == DestroyNotify) {
        RequestClose();
        continue;
      }
      if (event.type == ConfigureNotify) {
        const int new_width = std::max(1, event.xconfigure.width);
        const int new_height = std::max(1, event.xconfigure.height);
        if (new_width != width_ || new_height != height_) {
          Configure(new_width, new_height);
        }
        continue;
      }
      if (!browser_ref) {
        continue;
      }
      auto host = browser_ref->GetHost();
      if (event.type == Expose) {
        PresentLatest();
      } else if (event.type == MotionNotify) {
        CefMouseEvent mouse;
        mouse.x = event.xmotion.x;
        mouse.y = event.xmotion.y;
        mouse.modifiers = EventModifiers(event.xmotion.state);
        host->SendMouseMoveEvent(mouse, false);
      } else if (event.type == ButtonPress || event.type == ButtonRelease) {
        CefMouseEvent mouse;
        mouse.x = event.xbutton.x;
        mouse.y = event.xbutton.y;
        mouse.modifiers = EventModifiers(event.xbutton.state);
        if ((event.xbutton.button == Button4 || event.xbutton.button == Button5) &&
            event.type == ButtonPress) {
          const int delta = event.xbutton.button == Button4 ? 120 : -120;
          host->SendMouseWheelEvent(mouse, 0, delta);
        } else if (event.xbutton.button == Button1) {
          host->SendMouseClickEvent(mouse, MBT_LEFT, event.type == ButtonRelease, 1);
        } else if (event.xbutton.button == Button3) {
          host->SendMouseClickEvent(mouse, MBT_RIGHT, event.type == ButtonRelease, 1);
        }
      } else if (event.type == KeyPress) {
        DispatchKeyPress(host, event.xkey);
      }
    }
  }

  int width() const { return width_; }
  int height() const { return height_; }

 private:
  static int LowestBit(unsigned long mask) {
    if (mask == 0) {
      return 0;
    }
    int shift = 0;
    while ((mask & 1UL) == 0) {
      mask >>= 1;
      ++shift;
    }
    return shift;
  }

  static uint32_t EventModifiers(unsigned int state) {
    uint32_t modifiers = 0;
    if (state & ShiftMask) modifiers |= EVENTFLAG_SHIFT_DOWN;
    if (state & ControlMask) modifiers |= EVENTFLAG_CONTROL_DOWN;
    if (state & Mod1Mask) modifiers |= EVENTFLAG_ALT_DOWN;
    if (state & Button1Mask) modifiers |= EVENTFLAG_LEFT_MOUSE_BUTTON;
    if (state & Button2Mask) modifiers |= EVENTFLAG_MIDDLE_MOUSE_BUTTON;
    if (state & Button3Mask) modifiers |= EVENTFLAG_RIGHT_MOUSE_BUTTON;
    return modifiers;
  }

  static void DispatchKeyPress(CefRefPtr<CefBrowserHost> host, XKeyEvent& key) {
    const KeySym symbol = XLookupKeysym(&key, 0);
    if (symbol == XK_Escape || (symbol == XK_q && (key.state & ControlMask) == 0)) {
      g_close_requested.store(true);
      host->CloseBrowser(false);
      return;
    }

    char text[32] = {};
    KeySym translated = NoSymbol;
    const int length = XLookupString(&key, text, sizeof(text), &translated, nullptr);
    CefKeyEvent raw;
    raw.type = KEYEVENT_RAWKEYDOWN;
    raw.windows_key_code = key.keycode;
    raw.native_key_code = key.keycode;
    raw.modifiers = EventModifiers(key.state);
    host->SendKeyEvent(raw);
    for (int i = 0; i < length; ++i) {
      CefKeyEvent character;
      character.type = KEYEVENT_CHAR;
      character.windows_key_code = static_cast<unsigned char>(text[i]);
      character.native_key_code = key.keycode;
      character.character = static_cast<char16_t>(static_cast<unsigned char>(text[i]));
      character.unmodified_character = character.character;
      character.modifiers = raw.modifiers;
      host->SendKeyEvent(character);
    }
  }

  void EnsureImage(int width, int height) {
    if (image_ != nullptr && image_->width == width && image_->height == height) {
      return;
    }
    if (image_ != nullptr) {
      XDestroyImage(image_);
      image_ = nullptr;
    }
    if (display_ == nullptr || window_ == 0) {
      return;
    }
    XImage* image = XCreateImage(display_, DefaultVisual(display_, DefaultScreen(display_)),
                                 static_cast<unsigned>(DefaultDepth(display_, DefaultScreen(display_))),
                                 ZPixmap, 0, nullptr, static_cast<unsigned>(width),
                                 static_cast<unsigned>(height), 32, 0);
    if (image == nullptr) {
      return;
    }
    image->data = static_cast<char*>(std::calloc(1, static_cast<size_t>(image->bytes_per_line) * height));
    if (image->data == nullptr) {
      XDestroyImage(image);
      return;
    }
    image_ = image;
  }

  Display* display_ = nullptr;
  Window window_ = 0;
  Atom delete_atom_ = 0;
  XImage* image_ = nullptr;
  int width_ = kDefaultWidth;
  int height_ = kDefaultHeight;

  std::mutex browser_mutex_;
  CefRefPtr<CefBrowser> browser_;
  std::mutex frame_mutex_;
  std::vector<uint8_t> frame_;
  int frame_width_ = 0;
  int frame_height_ = 0;
  uint64_t frame_sequence_ = 0;
  bool frame_ready_ = false;
};

void PanelRenderHandler::GetViewRect(CefRefPtr<CefBrowser>, CefRect& rect) {
  rect = CefRect(0, 0, host_->width(), host_->height());
}

void PanelRenderHandler::OnPaint(CefRefPtr<CefBrowser>,
                                 PaintElementType type,
                                 const RectList&,
                                 const void* buffer,
                                 int width,
                                 int height) {
  if (type == PET_VIEW) {
    host_->SubmitFrame(buffer, width, height);
  }
}

class PanelClient final : public CefClient, public CefLifeSpanHandler {
 public:
  explicit PanelClient(HostWindow* host) : render_handler_(new PanelRenderHandler(host)) {}

  CefRefPtr<CefRenderHandler> GetRenderHandler() override { return render_handler_; }
  CefRefPtr<CefLifeSpanHandler> GetLifeSpanHandler() override { return this; }

  void OnAfterCreated(CefRefPtr<CefBrowser> browser) override {
    std::lock_guard<std::mutex> lock(mutex_);
    browser_ = browser;
    if (host_ != nullptr) {
      host_->SetBrowser(browser);
      if (host_->IsClosing()) {
        browser->GetHost()->CloseBrowser(false);
      }
    }
  }

  void OnBeforeClose(CefRefPtr<CefBrowser> browser) override {
    std::lock_guard<std::mutex> lock(mutex_);
    if (browser_ && browser_->GetIdentifier() == browser->GetIdentifier()) {
      browser_ = nullptr;
      if (host_ != nullptr) {
        host_->ClearBrowser();
      }
      g_done.store(true);
    }
  }

  CefRefPtr<CefBrowser> browser() {
    std::lock_guard<std::mutex> lock(mutex_);
    return browser_;
  }

  void SetHost(HostWindow* host) { host_ = host; }

 private:
  HostWindow* host_ = nullptr;
  CefRefPtr<PanelRenderHandler> render_handler_;
  CefRefPtr<CefBrowser> browser_;
  std::mutex mutex_;
  IMPLEMENT_REFCOUNTING(PanelClient);
};

class PanelApp final : public CefApp, public CefBrowserProcessHandler {
 public:
  PanelApp(HostWindow* host, CefRefPtr<PanelClient> client, Options options)
      : host_(host), client_(client), options_(std::move(options)) {}

  CefRefPtr<CefBrowserProcessHandler> GetBrowserProcessHandler() override { return this; }

  void OnBeforeCommandLineProcessing(const CefString& process_type,
                                     CefRefPtr<CefCommandLine> command_line) override {
    if (process_type.empty()) {
      command_line->AppendSwitch("disable-gpu");
    }
  }

  void OnContextInitialized() override {
    CEF_REQUIRE_UI_THREAD();
    if (host_ == nullptr || client_ == nullptr) {
      return;
    }
    CefWindowInfo window_info;
    window_info.SetAsWindowless(host_->window_handle());
    CefBrowserSettings browser_settings;
    browser_settings.windowless_frame_rate = 60;
    if (!CefBrowserHost::CreateBrowser(window_info, client_, FixtureUrl(options_),
                                       browser_settings, nullptr, nullptr)) {
      std::cerr << "CEF OSR POC: CefBrowserHost::CreateBrowser failed" << std::endl;
      g_done.store(true);
    }
  }

 private:
  HostWindow* host_;
  CefRefPtr<PanelClient> client_;
  Options options_;
  IMPLEMENT_REFCOUNTING(PanelApp);
};

}  // namespace

int main(int argc, char* argv[]) {
  CefMainArgs main_args(argc, argv);
  Options options = ParseOptions(main_args);
  std::error_code error;
  const auto absolute_cache_path = std::filesystem::absolute(options.cache_path, error);
  if (error) {
    std::cerr << "CEF OSR POC: cannot resolve cache path: " << error.message() << std::endl;
    return 1;
  }
  options.cache_path = absolute_cache_path.string();
  CefRefPtr<PanelApp> subprocess_app(new PanelApp(nullptr, nullptr, options));
  const int subprocess_exit_code = CefExecuteProcess(main_args, subprocess_app, nullptr);
  if (subprocess_exit_code >= 0) {
    return subprocess_exit_code;
  }

  std::filesystem::create_directories(options.cache_path, error);
  if (error) {
    std::cerr << "CEF OSR POC: cannot create cache path: " << error.message() << std::endl;
    return 1;
  }

  HostWindow host;
  if (!host.Create(kDefaultWidth, kDefaultHeight)) {
    return 1;
  }
  CefRefPtr<PanelClient> client(new PanelClient(&host));
  client->SetHost(&host);
  CefRefPtr<PanelApp> app(new PanelApp(&host, client, options));


  CefSettings settings;
  settings.no_sandbox = true;
  settings.windowless_rendering_enabled = true;
  settings.remote_debugging_port = options.remote_debugging_port;
  const auto executable_dir = ExecutableDirectory();
  CefString(&settings.resources_dir_path) = executable_dir.string();
  CefString(&settings.locales_dir_path) = (executable_dir / "locales").string();
  CefString(&settings.cache_path) = options.cache_path;
  settings.log_severity = LOGSEVERITY_WARNING;

  if (!CefInitialize(main_args, settings, app, nullptr)) {
    std::cerr << "CEF OSR POC: CefInitialize failed" << std::endl;
    return 1;
  }
  std::cout << "CDP endpoint http://127.0.0.1:" << options.remote_debugging_port << std::endl;
  std::cout << "Fixture " << FixtureUrl(options) << std::endl;

  while (!g_done.load()) {
    host.DispatchEvents();
    host.PresentLatest();
    CefDoMessageLoopWork();
    std::this_thread::sleep_for(std::chrono::milliseconds(8));
  }

  CefShutdown();
  return 0;
}
