use gpui::Global;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSBitmapImageRep, NSEventMask, NSImage, NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{NSData, NSRect, NSSize};

const ICON_1X: &[u8] = include_bytes!("../../../resources/trayTemplate.png");
const ICON_2X: &[u8] = include_bytes!("../../../resources/trayTemplate@2x.png");
const ICON_POINTS: f64 = 22.0;

/// The menu bar icon. Left and right click both call `on_click`.
pub struct StatusItem {
    item: Retained<NSStatusItem>,
    // The button holds its target weakly.
    target: Retained<ClickTarget>,
}

impl Global for StatusItem {}

impl StatusItem {
    pub fn new(mtm: MainThreadMarker, on_click: impl Fn() + 'static) -> Self {
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        let target = ClickTarget::new(mtm, Box::new(on_click));
        let button = item.button(mtm).expect("status items have a button");
        button.setImage(Some(&template_icon()));
        unsafe {
            button.setTarget(Some(&target));
            button.setAction(Some(sel!(click:)));
        }
        button.sendActionOn(NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown);
        Self { item, target }
    }

    /// The icon's frame in AppKit screen coordinates (origin bottom left).
    pub fn anchor(&self) -> Option<NSRect> {
        let mtm = MainThreadMarker::from(&*self.target);
        let button = self.item.button(mtm)?;
        Some(button.window()?.frame())
    }
}

fn template_icon() -> Retained<NSImage> {
    let size = NSSize::new(ICON_POINTS, ICON_POINTS);
    let image = NSImage::initWithSize(NSImage::alloc(), size);
    for png in [ICON_1X, ICON_2X] {
        let rep =
            NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &NSData::with_bytes(png))
                .expect("tray icons are valid PNGs");
        // The PNGs carry 72 dpi, so the 2x rep would otherwise claim 44 pt.
        rep.setSize(size);
        image.addRepresentation(&rep);
    }
    image.setTemplate(true);
    image
}

type Callback = Box<dyn Fn()>;

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SlopifyStatusItemTarget"]
    #[ivars = Callback]
    struct ClickTarget;

    impl ClickTarget {
        #[unsafe(method(click:))]
        fn click(&self, _sender: Option<&AnyObject>) {
            (self.ivars())();
        }
    }
);

impl ClickTarget {
    fn new(mtm: MainThreadMarker, callback: Callback) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(callback);
        unsafe { msg_send![super(this), init] }
    }
}
