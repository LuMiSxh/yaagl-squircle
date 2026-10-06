// SPDX-License-Identifier: MPL-2.0

#import <objc/message.h>
#import <objc/runtime.h>
#include <pthread.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

/*
 * Process-local Dock policy for Wine games started by YAAGL.
 *
 * The wrapper script installed by yaagl-squircle injects this dylib with
 * DYLD_INSERT_LIBRARIES. Wine later asks NSApplication to publish the icon it extracted
 * from the game executable. Intercepting that single public AppKit setter lets the bridge
 * clip the icon to a squircle on the macOS icon grid, or replace it with a user-supplied
 * PNG, without modifying Wine, its prefix, or the game executable.
 *
 * This file deliberately does not import or link AppKit/Foundation. Loading an AppKit-
 * linked injected dylib initializes AppKit before Wine has assigned
 * WINEPRELOADERAPPNAME, causing the menu bar to expose Wine's hosted-application name
 * instead of the game's. The bridge therefore waits until Wine itself has loaded
 * NSApplication and accesses the small required API surface through the Objective-C
 * runtime. All AppKit object creation and drawing still happens on Wine's calling
 * thread when it invokes setApplicationIconImage:; the polling thread only installs
 * the method hook.
 */

/* NSPoint, NSSize, and NSRect are intentionally mirrored instead of importing AppKit.
 * Their layouts are pairs of doubles, which must exactly match the objc_msgSend
 * function signatures below for struct arguments and return values to use the correct ABI. */
typedef struct {
	double x;
	double y;
} AKPoint;

typedef struct {
	double width;
	double height;
} AKSize;

typedef struct {
	AKPoint origin;
	AKSize size;
} AKRect;

typedef void (*SetApplicationIconImageIMP)(id, SEL, id);
typedef id (*RoundedPathIMP)(id, SEL, AKRect, double, double);

/* A 412-point content square inside a 512-point canvas is the 80.5% macOS icon grid; the
 * 92.3-point corner radius matches the rounded rectangle the Arknights launcher renders.
 * The integer enum values mirror NSCompositingOperationCopy and
 * NSImageInterpolationHigh; naming them here avoids loading AppKit merely to obtain
 * those declarations. */
static const double icon_canvas_dimension = 512.0;
static const double icon_content_dimension = 412.0;
static const double icon_corner_radius = 92.3;
static const long compositing_operation_copy = 1;
static const long image_interpolation_high = 3;

/* AppKit normally appears very early in Wine startup. Polling every 50ms for at most ten
 * seconds avoids blocking dyld's constructor thread while still installing the hook before
 * Wine publishes its executable icon, without keeping the CPU out of its idle states.
 * objc_getClass only observes runtime state; it does not cause AppKit to load. */
static const struct timespec appkit_poll_interval = { .tv_sec = 0, .tv_nsec = 50000000 };
static const int appkit_poll_limit = 200;

static SetApplicationIconImageIMP original_set_application_icon_image;
static pthread_mutex_t icon_setter_lock = PTHREAD_MUTEX_INITIALIZER;
static char *custom_game_icon_path;
static id custom_game_icon;
static bool debug_enabled;
static char *debug_log_path;

/* Debug output goes to stderr and, when the wrapper set YAAGL_SQUIRCLE_LOG, is appended to
 * that file with a timestamp and pid. The stderr line carries the "bridge loaded" marker
 * that the CLI's smoke test looks for, so it must stay even when a log file is set. */
static void debug_log(const char *format, ...) {
	if (!debug_enabled) return;

	char line[512];
	va_list arguments;
	va_start(arguments, format);
	vsnprintf(line, sizeof line, format, arguments);
	va_end(arguments);

	fprintf(stderr, "yaagl-squircle: %s\n", line);
	if (debug_log_path == NULL) return;
	FILE *file = fopen(debug_log_path, "a");
	if (file == NULL) return;
	fprintf(file, "%ld pid=%d %s\n", (long)time(NULL), (int)getpid(), line);
	fclose(file);
}

/* Typed objc_msgSend adapters are required because the runtime declares objc_msgSend
 * without the concrete return and argument ABI of each selector. Keep these signatures
 * aligned with the corresponding AppKit methods, especially the struct-valued variants. */
static id send_id(id receiver, SEL selector) {
	return ((id (*)(id, SEL))objc_msgSend)(receiver, selector);
}

static id send_id_with_id(id receiver, SEL selector, id value) {
	return ((id (*)(id, SEL, id))objc_msgSend)(receiver, selector, value);
}

static id send_id_with_size(id receiver, SEL selector, AKSize value) {
	return ((id (*)(id, SEL, AKSize))objc_msgSend)(receiver, selector, value);
}

static void send_void(id receiver, SEL selector) {
	((void (*)(id, SEL))objc_msgSend)(receiver, selector);
}

static void send_void_with_integer(id receiver, SEL selector, long value) {
	((void (*)(id, SEL, long))objc_msgSend)(receiver, selector, value);
}

static AKSize send_size(id receiver, SEL selector) {
	return ((AKSize (*)(id, SEL))objc_msgSend)(receiver, selector);
}

static void send_draw_message(
	id receiver, SEL selector, AKRect destination, AKRect source, long operation, double fraction) {
	((void (*)(id, SEL, AKRect, AKRect, long, double))objc_msgSend)(
		receiver, selector, destination, source, operation, fraction);
}

/* Resolves the optional user-supplied PNG only after Wine has initialized AppKit.
 * Both the copied C path and NSImage intentionally live for the remaining process
 * lifetime: Wine sets its application icon once, and retaining these objects prevents
 * an autorelease-pool boundary from invalidating the replacement before AppKit consumes it. */
static id load_custom_game_icon(void) {
	if (custom_game_icon != nil || custom_game_icon_path == NULL) return custom_game_icon;

	Class string_class = objc_getClass("NSString");
	Class image_class = objc_getClass("NSImage");
	if (string_class == Nil || image_class == Nil) return nil;

	id path = ((id (*)(id, SEL, const char *))objc_msgSend)(
		(id)string_class, sel_registerName("stringWithUTF8String:"), custom_game_icon_path);
	id image = send_id((id)image_class, sel_registerName("alloc"));
	custom_game_icon = send_id_with_id(image, sel_registerName("initWithContentsOfFile:"), path);
	if (custom_game_icon == nil) {
		fprintf(
			stderr, "yaagl-squircle: failed to load custom icon: %s\n", custom_game_icon_path);
	}
	return custom_game_icon;
}

/* Places Wine's full-bleed executable icon on a transparent 512x512 canvas, clipped to a
 * rounded square. Failures conservatively return the original NSImage so icon handling can
 * never prevent the game from presenting its Dock tile. Drawing uses the current AppKit
 * graphics context on Wine's setter-calling thread, not the polling thread. */
static id normalized_game_icon(id source) {
	if (source == nil) return nil;

	Class image_class = objc_getClass("NSImage");
	Class graphics_context_class = objc_getClass("NSGraphicsContext");
	if (image_class == Nil || graphics_context_class == Nil) {
		debug_log("NSImage/NSGraphicsContext missing; icon left unchanged");
		return source;
	}

	AKSize canvas_size = { icon_canvas_dimension, icon_canvas_dimension };
	id result = send_id((id)image_class, sel_registerName("alloc"));
	result = send_id_with_size(result, sel_registerName("initWithSize:"), canvas_size);
	if (result == nil) {
		debug_log("could not create the canvas image; icon left unchanged");
		return source;
	}

	send_void(result, sel_registerName("lockFocus"));
	id context = send_id((id)graphics_context_class, sel_registerName("currentContext"));
	send_void_with_integer(
		context, sel_registerName("setImageInterpolation:"), image_interpolation_high);

	double inset = (icon_canvas_dimension - icon_content_dimension) / 2.0;
	AKRect destination = { { inset, inset }, { icon_content_dimension, icon_content_dimension } };

	/* The clip lives in the image's focus context and ends with unlockFocus. A missing
	 * NSBezierPath only costs the rounded corners. */
	Class path_class = objc_getClass("NSBezierPath");
	if (path_class != Nil) {
		id path = ((RoundedPathIMP)objc_msgSend)(
			(id)path_class,
			sel_registerName("bezierPathWithRoundedRect:xRadius:yRadius:"),
			destination,
			icon_corner_radius,
			icon_corner_radius);
		if (path != nil) {
			send_void(path, sel_registerName("addClip"));
			debug_log("squircle clip applied");
		} else {
			debug_log("NSBezierPath returned nil; icon drawn without rounded corners");
		}
	} else {
		debug_log("NSBezierPath missing; icon drawn without rounded corners");
	}

	AKRect source_rect = { { 0.0, 0.0 }, send_size(source, sel_registerName("size")) };
	send_draw_message(
		source,
		sel_registerName("drawInRect:fromRect:operation:fraction:"),
		destination,
		source_rect,
		compositing_operation_copy,
		1.0);
	send_void(result, sel_registerName("unlockFocus"));
	return result;
}

/* Replacement IMP for NSApplication.setApplicationIconImage:. A valid custom image takes
 * precedence; a missing or unreadable one falls back to the squircle-normalized executable
 * icon. Calling the saved IMP preserves AppKit's normal Dock and application-icon side
 * effects. The pool catches AppKit's autoreleased temporaries, which no Wine-owned pool is
 * guaranteed to cover on the calling thread; both publishable icons are +1 and survive it. */
static void set_application_icon_image(id application, SEL selector, id image) {
	@autoreleasepool {
		SetApplicationIconImageIMP original;
		debug_log("setApplicationIconImage: called");
		id resolved = load_custom_game_icon();
		if (resolved != nil) {
			debug_log("using the custom icon");
		} else {
			resolved = normalized_game_icon(image);
		}
		pthread_mutex_lock(&icon_setter_lock);
		original = original_set_application_icon_image;
		pthread_mutex_unlock(&icon_setter_lock);
		if (original != NULL) original(application, selector, resolved);
	}
}

/* Installs the hook only after NSApplication exists. Holding the same mutex the replacement
 * uses while method_setImplementation publishes it prevents another AppKit caller from
 * observing the hook before its original target has been stored. */
static bool install_icon_setter(void) {
	Class application_class = objc_getClass("NSApplication");
	SetApplicationIconImageIMP original;
	if (application_class == Nil) return false;

	SEL selector = sel_registerName("setApplicationIconImage:");
	Method method = class_getInstanceMethod(application_class, selector);
	if (method == NULL) return false;

	pthread_mutex_lock(&icon_setter_lock);
	original = (SetApplicationIconImageIMP)method_setImplementation(
		method, (IMP)set_application_icon_image);
	original_set_application_icon_image = original;
	pthread_mutex_unlock(&icon_setter_lock);
	return original != NULL;
}

/* Background entry point used solely to observe AppKit availability and install the hook.
 * A timeout is non-fatal: the game keeps Wine's unmodified icon. */
static void *wait_for_appkit(void *context) {
	(void)context;
	for (int attempt = 0; attempt < appkit_poll_limit; attempt++) {
		@autoreleasepool {
			if (install_icon_setter()) {
				debug_log("hook installed after %d polls", attempt);
				return NULL;
			}
		}
		nanosleep(&appkit_poll_interval, NULL);
	}
	debug_log("timed out waiting for AppKit in %s", getprogname());
	return NULL;
}

/* The wrapper injects into every process Wine starts. Only Wine's own loader processes
 * (wine64, wine64.real, their preloaders) show a Dock tile; wineserver never loads AppKit. */
static bool is_wine_client_process(void) {
	const char *name = getprogname();
	return name != NULL && strncmp(name, "wine", 4) == 0 && strncmp(name, "wineserver", 10) != 0;
}

/* Dylib entry point. It snapshots the optional custom-icon path before returning from the
 * loader callback, then detaches the observer so dyld can continue Wine startup
 * immediately. Failures are deliberately non-fatal. */
__attribute__((constructor)) static void install_game_icon_bridge(void) {
	if (!is_wine_client_process()) return;

	debug_enabled = getenv("YAAGL_SQUIRCLE_DEBUG") != NULL;
	const char *log_path = getenv("YAAGL_SQUIRCLE_LOG");
	if (debug_enabled && log_path != NULL && log_path[0] != '\0') {
		debug_log_path = strdup(log_path);
		/* Several processes append to one file, so cap it by restarting it when large. */
		struct stat info;
		if (debug_log_path != NULL && stat(debug_log_path, &info) == 0 &&
			info.st_size > 512 * 1024) {
			truncate(debug_log_path, 0);
		}
	}
	debug_log("bridge loaded in %s", getprogname());

	const char *custom_path = getenv("YAAGL_SQUIRCLE_ICON");
	if (custom_path != NULL && custom_path[0] != '\0') {
		custom_game_icon_path = strdup(custom_path);
	}

	pthread_t thread;
	if (pthread_create(&thread, NULL, wait_for_appkit, NULL) == 0) {
		pthread_detach(thread);
	} else {
		debug_log("failed to start the bridge thread");
	}
}
