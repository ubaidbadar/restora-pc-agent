//! Non-blocking event *signals*. Windows notification callbacks only set an atomic bit;
//! actual Winspool/PnP discovery runs off the async runtime with a 30-second fallback.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_NOTIFY_ACTION, CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL,
    CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL, CM_NOTIFY_EVENT_DATA, CM_NOTIFY_FILTER,
    CM_NOTIFY_FILTER_0, CM_NOTIFY_FILTER_0_0, CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE,
    CM_Register_Notification, CM_Unregister_Notification, CR_SUCCESS, HCMNOTIFICATION,
};
use windows::Win32::Foundation::WAIT_OBJECT_0;
use windows::Win32::Graphics::Printing::{
    ClosePrinter, FindClosePrinterChangeNotification, FindFirstPrinterChangeNotification,
    FindNextPrinterChangeNotification, OpenPrinterW, PRINTER_CHANGE_JOB, PRINTER_CHANGE_PORT,
    PRINTER_CHANGE_PRINTER, PRINTER_HANDLE,
};
use windows::Win32::System::Threading::WaitForSingleObject;
use windows::core::PCWSTR;

unsafe extern "system" fn on_pnp(
    _handle: HCMNOTIFICATION,
    context: *const std::ffi::c_void,
    action: CM_NOTIFY_ACTION,
    _event: *const CM_NOTIFY_EVENT_DATA,
    _size: u32,
) -> u32 {
    if !context.is_null()
        && (action == CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL
            || action == CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL)
    {
        unsafe { &*(context.cast::<Arc<AtomicBool>>()) }.store(true, Ordering::Release);
    }
    0
}

/// One listener thread for local queue changes; a CM callback handles USB arrivals.
/// If registration fails, the backend's timed scan remains the authority.
pub fn start(pending: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let context = Box::into_raw(Box::new(pending.clone()));
        let filter = CM_NOTIFY_FILTER {
            cbSize: std::mem::size_of::<CM_NOTIFY_FILTER>() as u32,
            Flags: 0,
            FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE,
            Reserved: 0,
            u: CM_NOTIFY_FILTER_0 {
                DeviceInterface: CM_NOTIFY_FILTER_0_0 {
                    ClassGuid: crate::usb_pnp::USBPRINT_GUID,
                },
            },
        };
        let mut notification = HCMNOTIFICATION::default();
        let registered = unsafe {
            CM_Register_Notification(
                &filter,
                Some(context.cast()),
                Some(on_pnp),
                &mut notification,
            ) == CR_SUCCESS
        };
        if !registered {
            tracing::warn!("USBPRINT change notification unavailable; 30s polling remains active");
        }

        loop {
            let mut server = PRINTER_HANDLE::default();
            let opened = unsafe { OpenPrinterW(PCWSTR::null(), &mut server, None) };
            if let Err(error) = opened {
                tracing::warn!("local print-server notifications unavailable: {error}");
                std::thread::sleep(Duration::from_secs(30));
                continue;
            }
            let mask = PRINTER_CHANGE_PRINTER | PRINTER_CHANGE_PORT | PRINTER_CHANGE_JOB;
            let change = unsafe { FindFirstPrinterChangeNotification(server, mask, 0, None) };
            if change.is_invalid() {
                let _ = unsafe { ClosePrinter(server) };
                std::thread::sleep(Duration::from_secs(30));
                continue;
            }
            loop {
                let signaled = unsafe { WaitForSingleObject(change, 5000) };
                if signaled == WAIT_OBJECT_0 {
                    let mut flags = 0u32;
                    let ok = unsafe {
                        FindNextPrinterChangeNotification(change, Some(&mut flags), None, None)
                    };
                    pending.store(true, Ordering::Release);
                    if !ok.as_bool() {
                        break;
                    }
                } else if signaled.0 != 258 {
                    // WAIT_TIMEOUT (258) is normal. Any other result requires re-registering.
                    break;
                }
            }
            let _ = unsafe { FindClosePrinterChangeNotification(change) };
            let _ = unsafe { ClosePrinter(server) };
            pending.store(true, Ordering::Release);
            std::thread::sleep(Duration::from_secs(2));
        }

        // This process-long listener intentionally lives for the agent's whole lifetime.
        #[allow(unreachable_code)]
        {
            if registered {
                let _ = unsafe { CM_Unregister_Notification(notification) };
            }
            unsafe { drop(Box::from_raw(context)) };
        }
    });
}
