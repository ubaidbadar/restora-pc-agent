//! Present USB printer *interfaces*, not a mapping to print queues.
//! Never infer USB001 == a particular device interface (or READY) from this inventory.
pub const USBPRINT_GUID: windows::core::GUID =
    windows::core::GUID::from_u128(0x28d78fad_5a12_11d1_ae5b_0000f803a8c2);

pub fn list_present() -> Result<Vec<String>, String> {
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA,
        SP_DEVICE_INTERFACE_DETAIL_DATA_W, SetupDiDestroyDeviceInfoList,
        SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
    };
    use windows::core::PCWSTR;

    let set = unsafe {
        SetupDiGetClassDevsW(
            Some(&USBPRINT_GUID),
            PCWSTR::null(),
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    }
    .map_err(|e| format!("SetupDiGetClassDevsW: {e}"))?;

    let result = (|| {
        let mut result = Vec::new();
        for index in 0..u32::MAX {
            let mut data = SP_DEVICE_INTERFACE_DATA {
                cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                ..Default::default()
            };
            if let Err(e) = unsafe {
                SetupDiEnumDeviceInterfaces(set, None, &USBPRINT_GUID, index, &mut data)
            } {
                // HRESULT_FROM_WIN32(ERROR_NO_MORE_ITEMS=259)
                if e.code().0 as u32 & 0xFFFF == 259 {
                    break;
                }
                return Err(format!("SetupDiEnumDeviceInterfaces: {e}"));
            }
            let mut needed = 0u32;
            let _ = unsafe {
                SetupDiGetDeviceInterfaceDetailW(set, &data, None, 0, Some(&mut needed), None)
            };
            if needed < std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32
                || needed > 65_536
            {
                return Err("invalid USBPRINT interface detail length".into());
            }
            let words = (needed as usize).div_ceil(std::mem::size_of::<usize>());
            let mut storage = vec![0usize; words];
            let detail = storage.as_mut_ptr().cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
            unsafe { (*detail).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32 };
            unsafe {
                SetupDiGetDeviceInterfaceDetailW(set, &data, Some(detail), needed, None, None)
            }
            .map_err(|e| format!("SetupDiGetDeviceInterfaceDetailW: {e}"))?;

            let path_offset = std::mem::offset_of!(SP_DEVICE_INTERFACE_DETAIL_DATA_W, DevicePath);
            let total = needed as usize;
            let units = (total - path_offset) / 2;
            let path = unsafe {
                std::slice::from_raw_parts(storage.as_ptr().cast::<u8>().add(path_offset).cast::<u16>(), units)
            };
            let end = path.iter().position(|ch| *ch == 0).ok_or("unterminated device path")?;
            result.push(String::from_utf16(&path[..end]).map_err(|e| e.to_string())?);
        }
        Ok(result)
    })();

    let _ = unsafe { SetupDiDestroyDeviceInfoList(set) };
    result
}
