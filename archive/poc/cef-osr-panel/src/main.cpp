#include "include/cef_app.h"
#include "include/cef_browser.h"
#include "include/cef_client.h"
#include "include/cef_image.h"
#include "include/cef_render_handler.h"
#include "include/cef_values.h"
#include "include/wrapper/cef_helpers.h"

#include <charconv>
#include <condition_variable>
#include <atomic>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <mutex>
#include <optional>
#include <string>
#include <thread>
#include <utility>
#include <limits.h>
#include <unistd.h>
#include <vector>

namespace {

constexpr int kWidth = CEF_OSR_PANEL_DEFAULT_WIDTH;
constexpr int kHeight = CEF_OSR_PANEL_DEFAULT_HEIGHT;
constexpr int kDefaultPort = 9223;
constexpr size_t kMaxFrameBytes = 8 * 1024 * 1024;

std::atomic<bool> g_done{false};

struct Options {
  std::string fixture_url;
  std::string cache_path = "runtime/cache";
  int remote_debugging_port = kDefaultPort;
};

std::optional<std::string> ArgumentValue(const CefMainArgs& args, const std::string& prefix) {
  for (int index = 1; index < args.argc; ++index) {
    const std::string argument(args.argv[index]);
    if (argument.rfind(prefix, 0) == 0) return argument.substr(prefix.size());
  }
  return std::nullopt;
}

Options ParseOptions(const CefMainArgs& args) {
  Options options;
  if (const auto value = ArgumentValue(args, "--fixture=")) options.fixture_url = *value;
  if (const auto value = ArgumentValue(args, "--cache-path=")) options.cache_path = *value;
  if (const auto value = ArgumentValue(args, "--remote-debugging-port=")) {
    int port = 0;
    const auto result = std::from_chars(value->data(), value->data() + value->size(), port);
    if (result.ec == std::errc{} && result.ptr == value->data() + value->size() && port >= 1024 && port <= 65535) {
      options.remote_debugging_port = port;
    }
  }
  if (options.fixture_url.empty()) {
    options.fixture_url = "file://" + std::filesystem::absolute("fixture/index.html").string();
  }
  return options;
}

std::string Base64(const std::vector<uint8_t>& bytes) {
  static constexpr char alphabet[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  std::string encoded;
  encoded.reserve((bytes.size() + 2) / 3 * 4);
  for (size_t index = 0; index < bytes.size(); index += 3) {
    const uint32_t first = bytes[index];
    const uint32_t second = index + 1 < bytes.size() ? bytes[index + 1] : 0;
    const uint32_t third = index + 2 < bytes.size() ? bytes[index + 2] : 0;
    const uint32_t value = (first << 16) | (second << 8) | third;
    encoded.push_back(alphabet[(value >> 18) & 63]);
    encoded.push_back(alphabet[(value >> 12) & 63]);
    encoded.push_back(index + 1 < bytes.size() ? alphabet[(value >> 6) & 63] : '=');
    encoded.push_back(index + 2 < bytes.size() ? alphabet[value & 63] : '=');
  }
  return encoded;
}

struct Frame {
  uint64_t sequence = 0;
  int width = 0;
  int height = 0;
  std::vector<uint8_t> jpeg;
};
class FramePublisher {
 public:
  FramePublisher() : writer_(&FramePublisher::WriteLoop, this) {}
  ~FramePublisher() { Stop(); }

  void Submit(const void* buffer, int width, int height) {
    if (!buffer || width <= 0 || height <= 0) return;
    CefRefPtr<CefImage> image = CefImage::CreateImage();
    if (!image || !image->AddBitmap(1.0f, width, height, CEF_COLOR_TYPE_BGRA_8888,
                                    CEF_ALPHA_TYPE_OPAQUE, buffer,
                                    static_cast<size_t>(width) * height * 4)) {
      return;
    }
    int jpeg_width = 0;
    int jpeg_height = 0;
    CefRefPtr<CefBinaryValue> jpeg = image->GetAsJPEG(1.0f, 78, jpeg_width, jpeg_height);
    if (!jpeg) return;
    const size_t size = jpeg->GetSize();
    if (size == 0 || size > kMaxFrameBytes) return;
    Frame next;
    next.width = jpeg_width;
    next.height = jpeg_height;
    next.jpeg.resize(size);
    if (jpeg->GetData(next.jpeg.data(), size, 0) != size) return;
    {
      std::lock_guard<std::mutex> lock(mutex_);
      next.sequence = ++sequence_;
      latest_ = std::move(next);
    }
    condition_.notify_one();
  }

  void Stop() {
    {
      std::lock_guard<std::mutex> lock(mutex_);
      if (stopping_) return;
      stopping_ = true;
    }
    condition_.notify_one();
    if (writer_.joinable()) writer_.join();
  }

 private:
  void WriteLoop() {
    for (;;) {
      std::optional<Frame> next;
      {
        std::unique_lock<std::mutex> lock(mutex_);
        condition_.wait(lock, [this] { return stopping_ || latest_.has_value(); });
        if (stopping_ && !latest_) return;
        next = std::move(latest_);
        latest_.reset();
      }
      const std::string data = Base64(next->jpeg);
      std::cout << "{\"event\":\"frame\",\"frame\":{\"jpegDataUrl\":\"data:image/jpeg;base64,"
                << data << "\",\"sequence\":" << next->sequence << ",\"width\":"
                << next->width << ",\"height\":" << next->height << "}}" << std::endl;
    }
  }

  std::mutex mutex_;
  std::condition_variable condition_;
  std::optional<Frame> latest_;
  uint64_t sequence_ = 0;
  bool stopping_ = false;
  std::thread writer_;
};

class PanelRenderHandler final : public CefRenderHandler {
 public:
  explicit PanelRenderHandler(FramePublisher* publisher) : publisher_(publisher) {}

  void GetViewRect(CefRefPtr<CefBrowser>, CefRect& rect) override {
    rect = CefRect(0, 0, kWidth, kHeight);
  }

  void OnPaint(CefRefPtr<CefBrowser>, PaintElementType type, const RectList&, const void* buffer,
               int width, int height) override {
    if (type == PET_VIEW) publisher_->Submit(buffer, width, height);
  }

 private:
  FramePublisher* publisher_;
  IMPLEMENT_REFCOUNTING(PanelRenderHandler);
};

class PanelClient final : public CefClient, public CefLifeSpanHandler {
 public:
  explicit PanelClient(FramePublisher* publisher) : render_handler_(new PanelRenderHandler(publisher)) {}

  CefRefPtr<CefRenderHandler> GetRenderHandler() override { return render_handler_; }
  CefRefPtr<CefLifeSpanHandler> GetLifeSpanHandler() override { return this; }

  void OnAfterCreated(CefRefPtr<CefBrowser>) override {}

  void OnBeforeClose(CefRefPtr<CefBrowser>) override { g_done.store(true); }

 private:
  CefRefPtr<PanelRenderHandler> render_handler_;
  IMPLEMENT_REFCOUNTING(PanelClient);
};

class PanelApp final : public CefApp, public CefBrowserProcessHandler {
 public:
  PanelApp(CefRefPtr<PanelClient> client, Options options)
      : client_(std::move(client)), options_(std::move(options)) {}

  CefRefPtr<CefBrowserProcessHandler> GetBrowserProcessHandler() override { return this; }

  void OnBeforeCommandLineProcessing(const CefString& process_type,
                                     CefRefPtr<CefCommandLine> command_line) override {
    if (process_type.empty()) command_line->AppendSwitch("disable-gpu");
  }

  void OnContextInitialized() override {
    CEF_REQUIRE_UI_THREAD();
    if (!client_) return;
    CefWindowInfo window_info;
    window_info.SetAsWindowless(0);
    CefBrowserSettings settings;
    settings.windowless_frame_rate = 60;
    if (!CefBrowserHost::CreateBrowser(window_info, client_, options_.fixture_url, settings, nullptr, nullptr)) {
      std::cerr << "CEF OSR sidecar: browser creation failed" << std::endl;
      g_done.store(true);
    }
  }

 private:
  CefRefPtr<PanelClient> client_;
  Options options_;
  IMPLEMENT_REFCOUNTING(PanelApp);
};

}  // namespace

int main(int argc, char* argv[]) {
  CefMainArgs main_args(argc, argv);
  Options options = ParseOptions(main_args);
  std::error_code error;
  options.cache_path = std::filesystem::absolute(options.cache_path, error).string();
  if (error) {
    std::cerr << "CEF OSR sidecar: invalid cache path: " << error.message() << std::endl;
    return 1;
  }
  CefRefPtr<PanelApp> subprocess_app(new PanelApp(nullptr, options));
  const int subprocess_exit_code = CefExecuteProcess(main_args, subprocess_app, nullptr);
  if (subprocess_exit_code >= 0) return subprocess_exit_code;
  std::filesystem::create_directories(options.cache_path, error);
  if (error) {
    std::cerr << "CEF OSR sidecar: cannot create cache path: " << error.message() << std::endl;
    return 1;
  }

  FramePublisher publisher;
  CefRefPtr<PanelClient> client(new PanelClient(&publisher));
  CefRefPtr<PanelApp> app(new PanelApp(client, options));
  CefSettings settings;
  settings.no_sandbox = true;
  settings.windowless_rendering_enabled = true;
  settings.remote_debugging_port = options.remote_debugging_port;
  const auto executable_directory = [] {
    char path[PATH_MAX] = {};
    const ssize_t length = readlink("/proc/self/exe", path, sizeof(path) - 1);
    if (length > 0) {
      path[length] = '\0';
      return std::filesystem::path(path).parent_path();
    }
    return std::filesystem::current_path();
  }();
  CefString(&settings.resources_dir_path) = executable_directory.string();
  CefString(&settings.locales_dir_path) = (executable_directory / "locales").string();
  CefString(&settings.cache_path) = options.cache_path;
  settings.log_severity = LOGSEVERITY_WARNING;
  std::cout << "CDP endpoint http://127.0.0.1:" << options.remote_debugging_port << std::endl;

  if (!CefInitialize(main_args, settings, app, nullptr)) {
    std::cerr << "CEF OSR sidecar: CefInitialize failed" << std::endl;
    return 1;
  }
  while (!g_done.load()) {
    CefDoMessageLoopWork();
    std::this_thread::sleep_for(std::chrono::milliseconds(8));
  }
  CefShutdown();
  publisher.Stop();
  return 0;
}
